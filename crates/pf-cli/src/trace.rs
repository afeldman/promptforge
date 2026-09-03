//! Debug-Trace-Aufbereitung (CLI `--debug` / `--debug-json`).
//!
//! Die Engine emittiert pro LLM-Call ein `StageEvent::LlmTrace` mit den
//! tatsächlich gesendeten Prompt-Texten (Echo der Python-Schicht) und der
//! rohen Antwort (redigiert). Dieser Modul baut daraus ein stabiles,
//! serialisierbares Trace-Dokument:
//!
//! ```json
//! {
//!   "input": "…",
//!   "llm_used": true,
//!   "stages": [
//!     { "stage": "architect", "llm": true, "attempts": [ { "attempt": 1,
//!       "system_prompt": "…", "user_prompt": "…", "raw_response": "…",
//!       "duration_ms": … } ] },
//!     { "stage": "expand", "llm": false,
//!       "note": "deterministisch (kein LLM-Request)" },
//!     …
//!   ]
//! }
//! ```
//!
//! Stufen ohne LLM (z. B. `expand`, oder `--no-llm`) werden transparent mit
//! `llm: false` dargestellt — es wird NIE ein künstliches `raw_response`
//! erzeugt.

use std::collections::BTreeMap;

use pf_core::compilation::CompilationResult;
use pf_core::error::Result;
use pf_engine::StageEvent;
use serde::Serialize;

/// Ein LLM-Aufruf innerhalb einer Stufe (echte Werte aus dem Call-Pfad).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TraceAttempt {
    pub attempt: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    pub user_prompt: String,
    pub raw_response: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Eine Pipeline-Stufe im Trace.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TraceStage {
    pub stage: String,
    /// Hat diese Stufe LLM-Calls ausgeführt?
    pub llm: bool,
    /// Immer vorhanden (leer, wenn die Stufe keinen LLM-Call hatte).
    pub attempts: Vec<TraceAttempt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Vollständiges, serialisierbares Debug-Trace-Dokument.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TraceDoc {
    pub input: String,
    pub llm_used: bool,
    pub stages: Vec<TraceStage>,
    /// Pipeline-Notizen (z. B. Guard-Wiederherstellungen, Re-Optimize).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl TraceDoc {
    pub fn to_json_pretty(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map(|s| format!("{s}\n"))
            .map_err(Into::into)
    }
}

/// Baut das Trace-Dokument aus den erfassten Events + Ergebnis.
pub fn build_trace(outcome: &CompilationResult, events: &[StageEvent]) -> TraceDoc {
    build_trace_inner(Some(outcome), events)
}

/// Baut ein (partielles) Trace-Dokument auch ohne Ergebnis (Fehlerpfad).
pub fn build_partial_trace(input: &str, events: &[StageEvent]) -> TraceDoc {
    let mut doc = build_trace_inner(None, events);
    doc.input = input.to_string();
    doc
}

fn build_trace_inner(outcome: Option<&CompilationResult>, events: &[StageEvent]) -> TraceDoc {
    // LlmTrace-Events je Stufe sammeln (Reihenfolge = Versuchs-Reihenfolge).
    let mut by_stage: BTreeMap<&'static str, Vec<TraceAttempt>> = BTreeMap::new();
    for ev in events {
        if let StageEvent::LlmTrace {
            stage,
            attempt,
            system_prompt,
            user_prompt,
            raw_response,
            duration_ms,
        } = ev
        {
            by_stage
                .entry(stage.as_str())
                .or_default()
                .push(TraceAttempt {
                    attempt: *attempt,
                    system_prompt: system_prompt.clone(),
                    user_prompt: user_prompt.clone(),
                    raw_response: raw_response.clone(),
                    duration_ms: *duration_ms,
                });
        }
    }

    let llm_used = outcome.map(|o| o.llm_used).unwrap_or(true);
    let input = outcome
        .map(|o| o.input.clone())
        .unwrap_or_else(|| "?".to_string());

    let notes: Vec<String> = events
        .iter()
        .filter_map(|ev| match ev {
            StageEvent::Note(n) => Some(n.clone()),
            _ => None,
        })
        .collect();

    // Reihenfolge: Ergebnis-Stadien (falls vorhanden) sonst Event-Reihenfolge.
    let order: Vec<&str> = match outcome {
        Some(o) => o.stages.iter().map(String::as_str).collect(),
        None => {
            let mut seen = Vec::new();
            for ev in events {
                match ev {
                    StageEvent::StageStarted(s)
                    | StageEvent::StageFinished { stage: s, .. }
                    | StageEvent::LlmUsage { stage: s, .. }
                    | StageEvent::LlmTrace { stage: s, .. } => {
                        let name = s.as_str();
                        if !seen.contains(&name) {
                            seen.push(name);
                        }
                    }
                    StageEvent::Note(_) => {}
                }
            }
            seen
        }
    };

    let mut stages = Vec::new();
    for name in order {
        let attempts = by_stage.remove(name).unwrap_or_default();
        let llm = !attempts.is_empty();
        let note = if llm {
            None
        } else if name == "expand" {
            Some("deterministisch (kein LLM-Request)".to_string())
        } else if !llm_used {
            Some("kein LLM (deterministisch, --no-llm)".to_string())
        } else {
            Some("kein LLM-Request erfasst".to_string())
        };
        stages.push(TraceStage {
            stage: name.to_string(),
            llm,
            attempts,
            note,
        });
    }
    TraceDoc {
        input,
        llm_used,
        stages,
        notes,
    }
}

/// Kompakte menschlesbare Trace-Darstellung (stderr bei `--debug`).
/// Prompts/Responses werden abgeschnitten (Volltexte im JSON-Trace).
pub fn trace_human(doc: &TraceDoc) -> String {
    const SNIP: usize = 240;
    let mut out = String::new();
    out.push_str(&format!(
        "\n[trace] pipeline-trace input={:?} llm_used={}\n",
        doc.input, doc.llm_used
    ));
    for stage in &doc.stages {
        out.push_str(&format!(
            "  [trace] {:<10} llm={:<5} {}\n",
            stage.stage,
            if stage.llm { "true" } else { "false" },
            stage.note.as_deref().unwrap_or("")
        ));
        for at in &stage.attempts {
            let sys = at
                .system_prompt
                .as_deref()
                .map(|s| snippet(s, SNIP))
                .unwrap_or_else(|| "(kein System-Prompt)".to_string());
            out.push_str(&format!(
                "  [trace] {:<10} attempt={} system={}\n",
                "", at.attempt, sys
            ));
            out.push_str(&format!(
                "  [trace] {:<10}           user={}\n",
                "",
                snippet(&at.user_prompt, SNIP)
            ));
            out.push_str(&format!(
                "  [trace] {:<10}           raw={}\n",
                "",
                snippet(&at.raw_response, SNIP)
            ));
        }
    }
    out.push('\n');
    if !doc.notes.is_empty() {
        out.push_str("[trace] Notizen:\n");
        for n in &doc.notes {
            out.push_str(&format!("  [trace] note: {n}\n"));
        }
    }
    out.push('\n');
    out
}

fn snippet(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        let head: String = chars[..max].iter().collect();
        format!("{head}… (+{} Zeichen)", chars.len() - max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pf_engine::{Engine, EngineConfig, Stage};

    fn engine_cfg() -> EngineConfig {
        EngineConfig {
            llm: pf_core::config::LlmConfig {
                model: Some("mock".to_string()),
                ..Default::default()
            },
            verify: Default::default(),
        }
    }

    #[test]
    fn no_llm_trace_marks_all_stages_deterministic() {
        let engine = Engine::deterministic(engine_cfg());
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let events_cb = std::rc::Rc::clone(&events);
        let outcome = engine
            .compile(
                "Auditiere das Projekt",
                Some(Box::new(move |e| events_cb.borrow_mut().push(e))),
            )
            .unwrap();
        let doc = build_trace(&outcome, &events.borrow());
        assert_eq!(doc.input, "Auditiere das Projekt");
        assert!(!doc.llm_used);
        assert_eq!(doc.stages.len(), 4);
        for s in &doc.stages {
            assert!(!s.llm, "Stufe {} darf ohne LLM kein Attempt haben", s.stage);
            assert!(s.attempts.is_empty());
            assert!(s.note.is_some());
        }
        // Keine künstlichen raw_response-Werte.
        let json = doc.to_json_pretty().unwrap();
        assert!(!json.contains("raw_response\": \""));
    }

    #[test]
    fn mock_trace_contains_real_prompts_per_attempt() {
        use pf_core::config::ProviderKind;
        use pf_core::token::HeuristicTokenizer;
        use pf_engine::mock::MockBridge;
        let engine = Engine::new(
            Box::new(MockBridge::new()),
            std::sync::Arc::new(HeuristicTokenizer),
            engine_cfg(),
            ProviderKind::Mock,
        );
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let events_cb = std::rc::Rc::clone(&events);
        let outcome = engine
            .compile(
                "Analysiere fünf Papers",
                Some(Box::new(move |e| events_cb.borrow_mut().push(e))),
            )
            .unwrap();
        assert!(outcome.llm_used);
        let doc = build_trace(&outcome, &events.borrow());
        assert!(doc.llm_used);
        let stages: std::collections::HashMap<_, _> =
            doc.stages.iter().map(|s| (s.stage.clone(), s)).collect();
        // LLM-Stufen tragen echte Prompts + Antwort aus dem Call-Pfad.
        for name in ["architect", "optimize", "verify"] {
            let s = stages.get(name).unwrap_or_else(|| panic!("{name} fehlt"));
            assert!(s.llm, "{name} soll llm=true sein");
            assert!(!s.attempts.is_empty(), "{name}: Attempts fehlen");
            let at = &s.attempts[0];
            assert!(!at.user_prompt.is_empty(), "{name}: user_prompt fehlt");
            assert!(
                !at.raw_response.trim().is_empty(),
                "{name}: raw_response fehlt (echter Call-Pfad)"
            );
            assert_eq!(at.attempt, 1, "{name}: erster Attempt = 1");
        }
        // expand: deterministisch, kein Fake-raw_response.
        assert!(!stages["expand"].llm);
        assert!(stages["expand"].attempts.is_empty());
    }

    #[test]
    fn partial_trace_works_without_outcome() {
        let events = vec![
            StageEvent::StageStarted(Stage::Architect),
            StageEvent::LlmTrace {
                stage: Stage::Architect,
                attempt: 1,
                system_prompt: Some("sys".to_string()),
                user_prompt: "user".to_string(),
                raw_response: "{\"task\":\"x\"}".to_string(),
                duration_ms: Some(3),
            },
        ];
        let doc = build_partial_trace("auditiere", &events);
        assert_eq!(doc.input, "auditiere");
        assert_eq!(doc.stages.len(), 1);
        assert_eq!(doc.stages[0].attempts.len(), 1);
        assert_eq!(doc.stages[0].attempts[0].attempt, 1);
        let json = doc.to_json_pretty().unwrap();
        assert!(json.contains("\"system_prompt\": \"sys\""));
        assert!(json.contains("\"raw_response\": \"{\\\"task\\\":\\\"x\\\"}\""));
    }
}
