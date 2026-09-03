//! Pipeline-Orchestrierung (Spec §1/§6/§7/§22 Vertical Slice).
//!
//! ```text
//! intent → architect → PromptIR → expand → LongPrompt → optimize
//!        → optimiert → verify → Ergebnis (ggf. Re-Optimize, max N)
//! ```
//!
//! LLM-Aufrufe laufen über die `LlmBridge` (Python/Mock); ohne LLM ist die
//! Pipeline vollständig deterministisch. Jede Stufe erzeugt Events und
//! Token-Statistiken.

use std::sync::Arc;

use pf_core::bridge::{LlmBridge, LlmOperation, LlmRequest, LlmResponse, Usage};
use pf_core::compilation::{CompilationResult, QualityMetrics};
use pf_core::config::{LlmConfig, ProviderKind, VerifyConfig};
use pf_core::error::{ErrorKind, Result, err};
use pf_core::ir::PromptIr;
use pf_core::token::{TokenReport, Tokenizer};

use crate::expand::expand_to_long_prompt;
use crate::mock::MockBridge;
use crate::optimizer::{OptimizerEvent, deterministic_pass_chain, reinsert_missing_atoms};
use crate::verify::{
    SemanticAtoms, Verdict, VerificationReport, atoms_payload, mandatory_atoms, merge_semantic,
    parse_semantic_report_json, verify_structural,
};

/// Pipeline-Stadien.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    Architect,
    Expand,
    Optimize,
    Verify,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Architect => "architect",
            Stage::Expand => "expand",
            Stage::Optimize => "optimize",
            Stage::Verify => "verify",
        }
    }
}

/// Ereignisse für CLI/TUI/Service (Fortschritt, ohne Prompt-Inhalte).
#[derive(Debug, Clone)]
pub enum StageEvent {
    StageStarted(Stage),
    StageFinished {
        stage: Stage,
        ok: bool,
    },
    LlmUsage {
        stage: Stage,
        usage: Usage,
    },
    /// Vollständiger LLM-Call für Debug-Trace (`--debug`/`--debug-json`).
    /// Trägt die tatsächlich gesendeten Prompts (Echo der Python-Schicht)
    /// und die rohe Antwort — redigiert (keine Secrets).
    LlmTrace {
        stage: Stage,
        attempt: u32,
        system_prompt: Option<String>,
        user_prompt: String,
        raw_response: String,
        duration_ms: Option<u64>,
    },
    Note(String),
}

/// Konfiguration einer Compile-Ausführung (aus AppConfig abgeleitet).
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub llm: LlmConfig,
    pub verify: VerifyConfig,
}

impl EngineConfig {
    /// Ist ein LLM-Aufruf vorgesehen (Provider any_llm/mock)?
    pub fn llm_available(&self, provider: ProviderKind) -> bool {
        matches!(provider, ProviderKind::AnyLlm | ProviderKind::Mock) && self.llm.is_configured()
    }
}

/// Die PromptForge-Engine: eine Instanz pro Konfiguration.
pub struct Engine {
    bridge: Box<dyn LlmBridge>,
    tokenizer: Arc<dyn Tokenizer>,
    cfg: EngineConfig,
    provider: ProviderKind,
}

impl Engine {
    /// Neue Engine. `bridge` ist die LLM-Boundary (Python oder Mock);
    /// bei `provider = None` wird sie nicht aufgerufen.
    pub fn new(
        bridge: Box<dyn LlmBridge>,
        tokenizer: Arc<dyn Tokenizer>,
        cfg: EngineConfig,
        provider: ProviderKind,
    ) -> Self {
        Self {
            bridge,
            tokenizer,
            cfg,
            provider,
        }
    }

    /// Deterministische Engine ohne LLM (für Tests/Offline).
    pub fn deterministic(cfg: EngineConfig) -> Self {
        Self::new(
            Box::new(MockBridge::new()),
            Arc::new(pf_core::HeuristicTokenizer),
            cfg,
            ProviderKind::None,
        )
    }

    pub fn config(&self) -> &EngineConfig {
        &self.cfg
    }

    pub fn provider(&self) -> ProviderKind {
        self.provider
    }

    fn llm_on(&self) -> bool {
        self.cfg.llm_available(self.provider)
    }

    fn emit(cb: &mut Option<Box<dyn FnMut(StageEvent)>>, ev: StageEvent) {
        if let Some(f) = cb.as_mut() {
            f(ev);
        }
    }

    fn bridge_request(
        &self,
        op: LlmOperation,
        payload: serde_json::Value,
        request_id: &str,
    ) -> LlmRequest {
        LlmRequest {
            operation: op,
            system_prompt: None,
            user_prompt: payload.to_string(),
            output_schema: None,
            endpoint: self.cfg.llm.endpoint.clone(),
            api_key: self.cfg.llm.key.clone(),
            model: self.cfg.llm.model.clone(),
            temperature: self.cfg.llm.temperature,
            max_tokens: self.cfg.llm.max_tokens,
            provider: match self.provider {
                ProviderKind::AnyLlm => "any_llm".to_string(),
                ProviderKind::Mock => "mock".to_string(),
                _ => "auto".to_string(),
            },
            request_id: request_id.to_string(),
            timeout_s: Some(self.cfg.llm.timeout_s),
        }
    }

    fn call(
        &self,
        op: LlmOperation,
        payload: serde_json::Value,
        request_id: &str,
        attempt: u32,
        cb: &mut Option<Box<dyn FnMut(StageEvent)>>,
    ) -> Result<LlmResponse> {
        let req = self.bridge_request(op, payload, request_id);
        tracing::info!(operation = op.as_str(), request_id, model = ?req.model, attempt, "llm call");
        let resp = self.bridge.complete(&req)?;
        if let Some(usage) = resp.usage {
            Self::emit(
                cb,
                StageEvent::LlmUsage {
                    stage: stage_of(op),
                    usage,
                },
            );
        }
        // Debug-Trace-Event: echte Prompts (Echo der Python-Schicht) + rohe
        // Antwort, redigiert (Secret-Werte, Bearer-/sk-/key=-Muster).
        let secrets: Vec<String> = self.cfg.llm.key.clone().into_iter().collect();
        let san = |s: &str| pf_core::redact::sanitize_line(s, &secrets);
        Self::emit(
            cb,
            StageEvent::LlmTrace {
                stage: stage_of(op),
                attempt,
                system_prompt: resp.system_prompt.as_deref().map(san),
                user_prompt: san(resp.user_prompt.as_deref().unwrap_or(&req.user_prompt)),
                raw_response: san(&resp.content),
                duration_ms: resp.duration_ms,
            },
        );
        Ok(resp)
    }

    /// Kompiliert einen Intent (natürliche Sprache) bis zum optimierten Prompt.
    /// `cb` erhält Fortschritts-Events (z. B. für TUI/CLI).
    ///
    /// Ergebnis ist ein formatneutrales `CompilationResult` (v0.2) — die
    /// einzige Ergebnisstruktur; keine zweite Pipeline.
    pub fn compile(
        &self,
        intent: &str,
        mut cb: Option<Box<dyn FnMut(StageEvent)>>,
    ) -> Result<CompilationResult> {
        let request_id = pf_core::path::request_id();
        let intent = intent.trim();
        if intent.is_empty() {
            return Err(err(ErrorKind::InvalidInput, "Intent ist leer"));
        }

        // ---- Stage 1: Architect ----
        Self::emit(&mut cb, StageEvent::StageStarted(Stage::Architect));
        let ir = if self.llm_on() {
            let payload = serde_json::json!({ "intent": intent });
            let resp = self.call(LlmOperation::Architect, payload, &request_id, 1, &mut cb)?;
            parse_ir(&resp, "Architect")?
        } else {
            PromptIr::from_intent_basic(intent, &request_id)
        };
        Self::emit(
            &mut cb,
            StageEvent::StageFinished {
                stage: Stage::Architect,
                ok: true,
            },
        );

        // ---- Stage 2: Expand (deterministisch, Rust) ----
        Self::emit(&mut cb, StageEvent::StageStarted(Stage::Expand));
        let long_prompt = expand_to_long_prompt(&ir);
        Self::emit(
            &mut cb,
            StageEvent::StageFinished {
                stage: Stage::Expand,
                ok: true,
            },
        );

        // ---- Stage 3/4: Optimize + Verify (mit Re-Optimize-Loop) ----
        let (optimized, verification, _optimizer_events) =
            self.optimize_and_verify(&ir, &long_prompt, &request_id, &mut cb)?;

        match verification.verdict {
            Some(Verdict::Pass) => {
                Self::emit(
                    &mut cb,
                    StageEvent::Note("Verifikation bestanden".to_string()),
                );
                Ok(self.finish(
                    intent,
                    ir,
                    &long_prompt,
                    &optimized,
                    verification,
                    request_id,
                ))
            }
            _ => {
                Self::emit(
                    &mut cb,
                    StageEvent::Note(format!(
                        "Verifikation fehlgeschlagen (semantic_preservation={:.2})",
                        verification.semantic_preservation
                    )),
                );
                Err(err(
                    ErrorKind::Verification,
                    format!(
                        "Verifikation nach {} Versuchen nicht bestanden (semantic_preservation={:.2})",
                        self.cfg.verify.max_attempts + 1,
                        verification.semantic_preservation
                    ),
                ))
            }
        }
    }

    /// Optimize- + Verify-Loop über eine IR + Long Prompt (Re-Optimize mit
    /// Feedback, begrenzt durch `verify.max_attempts`). Gibt den optimierten
    /// Prompt, den Verifikationsbericht (ggf. mit Fail-Verdict) und die
    /// Optimizer-Events zurück.
    pub fn optimize_and_verify(
        &self,
        ir: &PromptIr,
        long_prompt: &str,
        request_id: &str,
        cb: &mut Option<Box<dyn FnMut(StageEvent)>>,
    ) -> Result<(String, VerificationReport, Vec<OptimizerEvent>)> {
        Self::emit(cb, StageEvent::StageStarted(Stage::Optimize));
        let atoms = SemanticAtoms::from_ir(ir);
        let mut optimizer_events: Vec<OptimizerEvent> = Vec::new();
        let mut last_feedback: Vec<String> = Vec::new();
        let mut last_report = VerificationReport::default();

        for attempt in 0..=self.cfg.verify.max_attempts {
            let mut optimized = if self.llm_on() {
                let payload = serde_json::json!({
                    "ir": ir,
                    "long_prompt": long_prompt,
                    "feedback": last_feedback,
                });
                let resp = self.call(
                    LlmOperation::Optimize,
                    payload,
                    request_id,
                    attempt as u32 + 1,
                    cb,
                )?;
                extract_prompt(&resp)?
            } else {
                long_prompt.to_string()
            };
            // Deterministische Hygiene-Passes (auch nach LLM-Pass).
            let (hygiene, evs) = deterministic_pass_chain(&optimized);
            optimizer_events.extend(evs);
            // Guard-Pass: Pflicht-Atome (Constraints/Contract/Req. u. a.) erhalten.
            let (guarded, ev) = reinsert_missing_atoms(&hygiene, &mandatory_atoms(&atoms));
            if let crate::optimizer::PassAction::ReinsertedAtoms(n) = ev.action
                && n > 0
            {
                Self::emit(
                    cb,
                    StageEvent::Note(format!(
                        "Guard-Pass hat {n} verlorene Pflicht-Atome wiederhergestellt (Re-Optimize-Ergebnis war unvollständig)"
                    )),
                );
            }
            optimizer_events.push(ev);
            optimized = guarded;

            // ---- Verify ----
            Self::emit(cb, StageEvent::StageStarted(Stage::Verify));
            let structural = verify_structural(ir, &optimized, &atoms, self.cfg.verify.threshold);
            let mut report = if self.llm_on() {
                let payload = serde_json::json!({
                    "atoms": atoms_payload(&atoms),
                    "long_prompt": long_prompt,
                    "optimized_prompt": optimized,
                });
                let resp = self.call(
                    LlmOperation::Verify,
                    payload,
                    request_id,
                    attempt as u32 + 1,
                    cb,
                )?;
                let semantic = parse_semantic_report_json(&resp.parse_content_json()?)?;
                merge_semantic(structural, semantic, self.cfg.verify.threshold)
            } else {
                structural
            };
            report.attempts = (attempt + 1) as u32;
            last_report = report.clone();

            if report.verdict == Some(Verdict::Pass) {
                Self::emit(
                    cb,
                    StageEvent::StageFinished {
                        stage: Stage::Verify,
                        ok: true,
                    },
                );
                return Ok((optimized, report, optimizer_events));
            }
            Self::emit(
                cb,
                StageEvent::StageFinished {
                    stage: Stage::Verify,
                    ok: false,
                },
            );

            // Re-Optimize-Feedback aus fehlgeschlagenen Checks ableiten.
            last_feedback = failed_checks_feedback(&report);
            if !self.llm_on() || attempt >= self.cfg.verify.max_attempts {
                break;
            }
            Self::emit(
                cb,
                StageEvent::Note(format!(
                    "Re-Optimize ({}/{})",
                    attempt + 1,
                    self.cfg.verify.max_attempts
                )),
            );
        }

        // Nach max. Versuchen: Ergebnis mit Fail-Verdict (kein Abbruchfehler).
        Ok((long_prompt.to_string(), last_report, optimizer_events))
    }

    /// Einzelne Verifikation (strukturell + optional LLM-Semantik) ohne
    /// Re-Optimize-Loop — für `POST /v1/verify`.
    pub fn verify_pair(
        &self,
        ir: &PromptIr,
        long_prompt: &str,
        optimized: &str,
        request_id: &str,
        cb: &mut Option<Box<dyn FnMut(StageEvent)>>,
    ) -> Result<VerificationReport> {
        Self::emit(cb, StageEvent::StageStarted(Stage::Verify));
        let atoms = SemanticAtoms::from_ir(ir);
        let structural = verify_structural(ir, optimized, &atoms, self.cfg.verify.threshold);
        let report = if self.llm_on() {
            let payload = serde_json::json!({
                "atoms": atoms_payload(&atoms),
                "long_prompt": long_prompt,
                "optimized_prompt": optimized,
            });
            let resp = self.call(LlmOperation::Verify, payload, request_id, 1, cb)?;
            let semantic = parse_semantic_report_json(&resp.parse_content_json()?)?;
            merge_semantic(structural, semantic, self.cfg.verify.threshold)
        } else {
            structural
        };
        Self::emit(
            cb,
            StageEvent::StageFinished {
                stage: Stage::Verify,
                ok: report.verdict == Some(Verdict::Pass),
            },
        );
        Ok(report)
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        intent: &str,
        ir: PromptIr,
        long_prompt: &str,
        optimized: &str,
        verification: VerificationReport,
        request_id: String,
    ) -> CompilationResult {
        let tokenizer = Arc::clone(&self.tokenizer);
        let mut token_report = TokenReport::new();
        token_report.estimate = tokenizer.is_estimate();
        token_report.set_original(tokenizer.count(intent));
        token_report.set_generated(tokenizer.count(long_prompt));
        token_report.set_optimized(tokenizer.count(optimized));
        let metrics = QualityMetrics::compute(&verification, &token_report);
        CompilationResult {
            input: intent.to_string(),
            request_id,
            llm_used: self.llm_on(),
            stages: vec![
                Stage::Architect.as_str().to_string(),
                Stage::Expand.as_str().to_string(),
                Stage::Optimize.as_str().to_string(),
                Stage::Verify.as_str().to_string(),
            ],
            prompt_ir: ir,
            expanded_prompt: long_prompt.to_string(),
            optimized_prompt: optimized.to_string(),
            token_report,
            verification,
            metrics,
        }
    }
}

fn stage_of(op: LlmOperation) -> Stage {
    match op {
        LlmOperation::Architect => Stage::Architect,
        LlmOperation::Optimize => Stage::Optimize,
        LlmOperation::Verify => Stage::Verify,
        LlmOperation::Chat => Stage::Optimize,
    }
}

fn parse_ir(resp: &LlmResponse, stage: &str) -> Result<PromptIr> {
    let value = resp.parse_content_json().map_err(|_| {
        err(
            ErrorKind::Model,
            format!("{stage}: LLM-Antwort ist kein valides JSON"),
        )
    })?;
    PromptIr::from_json(&serde_json::to_string(&value)?)
        .map_err(|e| err(ErrorKind::Model, format!("{stage}: {e}")))
}

/// Extrahiert den Prompt aus der Optimize-Antwort (JSON `{prompt, notes}`
/// oder reiner Text).
fn extract_prompt(resp: &LlmResponse) -> Result<String> {
    match serde_json::from_str::<serde_json::Value>(resp.content.trim()) {
        Ok(v) => {
            if let Some(p) = v.get("prompt").and_then(|p| p.as_str()) {
                return Ok(p.to_string());
            }
            if let Some(s) = v.as_str() {
                return Ok(s.to_string());
            }
            Err(err(
                ErrorKind::Model,
                "Optimize: Antwort-JSON ohne 'prompt'-Feld",
            ))
        }
        Err(_) => Ok(resp.content.clone()),
    }
}

fn failed_checks_feedback(report: &VerificationReport) -> Vec<String> {
    report
        .checks
        .iter()
        .filter(|c| !c.ok)
        .map(|c| format!("[{}] {}", c.category, c.atom))
        .collect()
}

/// Führt einen Prompt gegen das Ziel-LLM aus (z. B. `POST /v1/execute`).
pub fn execute_prompt(
    engine: &Engine,
    prompt: &str,
    mut cb: Option<Box<dyn FnMut(StageEvent)>>,
) -> Result<LlmResponse> {
    if !engine.llm_on() {
        return Err(err(
            ErrorKind::Configuration,
            "Kein LLM konfiguriert (LLM_ENDPOINT/LLM_MODEL) — Ausführung nicht möglich",
        ));
    }
    let request_id = pf_core::path::request_id();
    let payload = serde_json::json!({ "prompt": prompt });
    engine.call(LlmOperation::Chat, payload, &request_id, 1, &mut cb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pf_core::config::VerifyConfig;

    fn cfg() -> EngineConfig {
        EngineConfig {
            llm: LlmConfig {
                model: Some("mock-model".to_string()),
                ..Default::default()
            },
            verify: VerifyConfig::default(),
        }
    }

    #[test]
    fn deterministic_compile_works_end_to_end() {
        let engine = Engine::deterministic(cfg());
        let outcome = engine
            .compile(
                "Analysiere diese fünf Papers und vergleiche die Methoden",
                None,
            )
            .unwrap();
        // v0.2: CompilationResult mit kanonischen Feldern + Metriken.
        assert_eq!(
            outcome.input,
            "Analysiere diese fünf Papers und vergleiche die Methoden"
        );
        assert!(outcome.expanded_prompt.contains("## Aufgabe"));
        assert!(!outcome.optimized_prompt.is_empty());
        assert!(!outcome.prompt_ir.task.is_empty());
        assert_eq!(outcome.verification.verdict, Some(Verdict::Pass));
        assert!(!outcome.llm_used);
        assert!(outcome.token_report.generated > 0);
        assert_eq!(outcome.stages.len(), 4);
        // Qualitätsmetriken deterministisch vorhanden.
        assert!(outcome.metrics.structural_validity);
        assert!(
            (outcome.metrics.semantic_fidelity - outcome.verification.semantic_preservation).abs()
                < 1e-9
        );
        assert!(outcome.metrics.token_efficiency >= 0.0); // ohne LLM: 1:1-Kopie
    }

    #[test]
    fn deterministic_compile_input_is_trimmed_and_analysis_defaults() {
        let engine = Engine::deterministic(cfg());
        let outcome = engine.compile("  Auditiere das Projekt  ", None).unwrap();
        assert_eq!(outcome.input, "Auditiere das Projekt");
        assert!(outcome.prompt_ir.analysis.is_none());
        assert!(outcome.prompt_ir.metadata.tags.is_empty());
        // CompilationResult ist JSON-serialisierbar (formatneutral).
        let json = serde_json::to_string(&outcome).unwrap();
        let back: pf_core::CompilationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, outcome);
    }

    #[test]
    fn mock_compile_works_end_to_end() {
        let engine = Engine::new(
            Box::new(MockBridge::new()),
            Arc::new(pf_core::HeuristicTokenizer),
            cfg(),
            ProviderKind::Mock,
        );
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let events_cb = std::rc::Rc::clone(&events);
        let outcome = engine
            .compile(
                "Analysiere fünf Papers und finde Widersprüche",
                Some(Box::new(move |e| events_cb.borrow_mut().push(e))),
            )
            .unwrap();
        assert!(outcome.llm_used);
        assert_eq!(outcome.verification.verdict, Some(Verdict::Pass));
        assert!(outcome.prompt_ir.role.is_some());
        // CompilationResult vorhanden (v0.2) — echter Pipeline-Lauf.
        assert!(!outcome.input.is_empty());
        assert_eq!(outcome.stages.len(), 4);
        assert!(outcome.metrics.semantic_fidelity >= 0.0);
        // Alle Stadien wurden als Started/Finished gemeldet.
        let starts = events
            .borrow()
            .iter()
            .filter(|e| matches!(e, StageEvent::StageStarted(_)))
            .count();
        assert!(starts >= 4, "starts={starts}");
        let _ = outcome;
    }

    #[test]
    fn empty_intent_is_rejected() {
        let engine = Engine::deterministic(cfg());
        assert!(engine.compile("   ", None).is_err());
    }

    #[test]
    fn execute_without_llm_errors() {
        let engine = Engine::deterministic(cfg());
        let e = execute_prompt(&engine, "hallo", None).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Configuration);
    }

    #[test]
    fn mock_execute_returns_content() {
        let engine = Engine::new(
            Box::new(MockBridge::new()),
            Arc::new(pf_core::HeuristicTokenizer),
            cfg(),
            ProviderKind::Mock,
        );
        let resp = execute_prompt(&engine, "Fasse zusammen", None).unwrap();
        assert!(resp.content.starts_with("Mock-Antwort"));
    }
}
