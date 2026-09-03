//! E2E-Integrationstest: Rust → PyO3 → Python (`promptforge.bridge`) →
//! Mock-LLM → Python → Rust (Spec §18/§22 Vertical Slice).
//!
//! Voraussetzung: Python-Paket `promptforge` importierbar (automatische
//! Repo-Suche über `python/src` oder `PF_PYTHON_PATH`). Ohne Python wird
//! der Test mit Hinweis übersprungen (kein Fehlschlag in CI ohne Venv).

use std::sync::Arc;

use pf_core::config::{AppConfig, ProviderKind};
use pf_core::error::Result;
use pf_core::token::HeuristicTokenizer;
use pf_engine::{Engine, EngineConfig, Verdict};

fn engine_mock_py(cfg: &AppConfig) -> Result<Engine> {
    for p in &cfg.python_paths {
        pf_bridge::push_extra_path(p.clone());
    }
    Ok(Engine::new(
        Box::new(pf_bridge::PythonBridge),
        Arc::new(HeuristicTokenizer),
        EngineConfig {
            llm: cfg.llm.clone(),
            verify: cfg.verify.clone(),
        },
        ProviderKind::Mock,
    ))
}

#[test]
fn e2e_rust_pyo3_python_mockllm() {
    // Python-Verfügbarkeit prüfen (Skip mit Hinweis, wenn nicht importierbar).
    if let Err(e) = pf_bridge::python::ensure_initialized() {
        eprintln!("SKIP e2e_rust_pyo3_python_mockllm: Python-Bridge nicht verfügbar ({e})");
        return;
    }
    if pf_bridge::python::call_json(r#"{"operation":"chat","user_prompt":"{}","provider":"mock"}"#)
        .is_err()
    {
        eprintln!("SKIP e2e_rust_pyo3_python_mockllm: promptforge-Paket nicht importierbar");
        return;
    }

    let home = std::env::temp_dir().join(format!("pf-e2e-{}", uuid_lite()));
    let mut cfg = AppConfig::load(Some(&home)).unwrap();
    cfg.provider = ProviderKind::Mock;
    cfg.llm.model = Some("mock-model".to_string());

    let engine = engine_mock_py(&cfg).unwrap();
    // Events sammeln (inkl. LlmTrace für den Debug-Trace).
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let events_cb = std::rc::Rc::clone(&events);
    let outcome = engine
        .compile(
            "Analysiere diese fünf Papers und vergleiche die Methoden",
            Some(Box::new(move |e| events_cb.borrow_mut().push(e))),
        )
        .unwrap();

    // Pipeline lief über Python (Mock) — nicht deterministisch.
    assert!(outcome.llm_used, "erwartet: Python-Mock-LLM verwendet");
    // Python-Mock setzt eine Rolle im IR.
    assert!(
        outcome
            .prompt_ir
            .role
            .as_deref()
            .unwrap_or("")
            .contains("Mock")
    );
    // Verifikation bestanden (strukturell + Python-Semantik-Report).
    assert_eq!(outcome.verification.verdict, Some(Verdict::Pass));
    assert!(outcome.expanded_prompt.contains("## Aufgabe"));
    assert!(!outcome.optimized_prompt.is_empty());
    assert!(outcome.token_report.generated > 0);
    assert_eq!(outcome.stages.len(), 4);
    // v0.2: CompilationResult ist das echte Engine-Ergebnis (kein Fake).
    assert_eq!(
        outcome.input,
        "Analysiere diese fünf Papers und vergleiche die Methoden"
    );
    assert!(outcome.metrics.semantic_fidelity >= 0.0);
    let envelope = outcome.envelope_json();
    assert!(envelope.get("metrics").is_some());
    assert!(envelope.get("prompt_ir").is_some());
    assert!(envelope.get("ir").is_some()); // v0.1-Alias bleibt erhalten

    // v0.2 Debug-Trace: echte Prompts/Antworten aus dem Python-Mock-Pfad.
    let trace = pf_cli::trace::build_trace(&outcome, &events.borrow());
    let stage_map: std::collections::HashMap<_, _> =
        trace.stages.iter().map(|s| (s.stage.as_str(), s)).collect();
    for name in ["architect", "optimize", "verify"] {
        let s = stage_map
            .get(name)
            .unwrap_or_else(|| panic!("{name} fehlt"));
        assert!(s.llm, "{name}: soll LLM-Calls tragen");
        assert!(!s.attempts.is_empty(), "{name}: keine Attempts erfasst");
        let at = &s.attempts[0];
        // Werte stammen aus dem tatsächlichen Request/Response-Pfad: Die
        // Python-Schicht echoed die tatsächlich verwendeten Prompts.
        assert!(
            at.system_prompt.as_deref().unwrap_or("").len() > 10,
            "{name}: system_prompt fehlt (Python-Echo)"
        );
        assert!(
            at.user_prompt.trim().len() > 10,
            "{name}: user_prompt fehlt (Python-Echo)"
        );
        assert!(
            at.raw_response.trim().len() > 1,
            "{name}: raw_response fehlt (echte Antwort)"
        );
        assert_eq!(at.attempt, 1);
    }
    // expand: deterministisch, kein Fake-raw_response.
    assert!(!stage_map["expand"].llm);
    assert!(stage_map["expand"].attempts.is_empty());

    let _ = std::fs::remove_dir_all(&home);
}

fn uuid_lite() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{n:x}")
}
