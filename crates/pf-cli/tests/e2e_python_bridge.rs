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
    let outcome = engine
        .compile(
            "Analysiere diese fünf Papers und vergleiche die Methoden",
            None,
        )
        .unwrap();

    // Pipeline lief über Python (Mock) — nicht deterministisch.
    assert!(outcome.llm_used, "erwartet: Python-Mock-LLM verwendet");
    // Python-Mock setzt eine Rolle im IR.
    assert!(outcome.ir.role.as_deref().unwrap_or("").contains("Mock"));
    // Verifikation bestanden (strukturell + Python-Semantik-Report).
    assert_eq!(outcome.verification.verdict, Some(Verdict::Pass));
    assert!(outcome.long_prompt.contains("## Aufgabe"));
    assert!(!outcome.optimized_prompt.is_empty());
    assert!(outcome.token_report.generated > 0);
    assert_eq!(outcome.stages_done.len(), 4);

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
