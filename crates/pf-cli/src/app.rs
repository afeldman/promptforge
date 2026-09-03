//! Anwendungslogik des `prompt-forge`-Binaries: init, Engine-Aufbau,
//! Kompilieren + Persistenz + History. CLI/TUI/Service nutzen diese
//! Funktionen — keine doppelte Business-Logik.

use std::path::PathBuf;
use std::sync::Arc;

use pf_core::config::{AppConfig, ProviderKind};
use pf_core::error::{ErrorKind, Result, err};
use pf_core::ir::PromptIr;
use pf_core::path::HomeLayout;
use pf_core::token::HeuristicTokenizer;
use pf_engine::{CompilationResult, Engine, EngineConfig, Stage, StageEvent, Verdict};

/// Ergebnis eines Kompilier-Laufs inkl. optionaler Persistenz.
#[derive(Debug, Clone)]
pub struct SaveResult {
    pub ir_path: PathBuf,
    pub long_prompt_path: PathBuf,
    pub optimized_path: PathBuf,
    pub history_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct CompileReport {
    pub outcome: CompilationResult,
    pub saved: Option<SaveResult>,
}

/// `prompt-forge init`: legt das User-Home an und schreibt eine
/// Default-Konfiguration (überschreibt nichts Bestehendes).
pub fn run_init(cfg: &AppConfig) -> Result<Vec<String>> {
    let layout = HomeLayout::from_root(&cfg.home);
    layout.ensure()?;
    let mut created = vec![format!("User-Home: {}", cfg.home.display())];
    let config_file = layout.config_dir.join("config.toml");
    if !config_file.exists() {
        pf_core::persist::atomic_write(&config_file, pf_core::DEFAULT_CONFIG_TOML.as_bytes())?;
        created.push(format!("Config: {}", config_file.display()));
    } else {
        created.push(format!(
            "Config vorhanden (unverändert): {}",
            config_file.display()
        ));
    }
    created.push(format!("Layout: {}/config, prompt/{{templates,generated,optimized,archive}}, history, cache, logs, state", cfg.home.display()));
    Ok(created)
}

/// EngineConfig aus AppConfig ableiten.
pub fn engine_config(cfg: &AppConfig) -> EngineConfig {
    EngineConfig {
        llm: cfg.llm.clone(),
        verify: cfg.verify.clone(),
    }
}

/// Engine passend zur effektiven Provider-Konfiguration bauen.
pub fn build_engine(cfg: &AppConfig) -> Result<Engine> {
    let provider = cfg.effective_provider();
    match provider {
        ProviderKind::None => Ok(Engine::deterministic(engine_config(cfg))),
        ProviderKind::AnyLlm | ProviderKind::Mock => {
            for p in &cfg.python_paths {
                pf_bridge::push_extra_path(p.clone());
            }
            let bridge = pf_bridge::PythonBridge;
            Ok(Engine::new(
                Box::new(bridge),
                Arc::new(HeuristicTokenizer),
                engine_config(cfg),
                provider,
            ))
        }
        ProviderKind::Auto => Err(err(ErrorKind::Configuration, "Provider auflösbar")),
    }
}

/// Kompiliert einen Intent und persistiert Artefakte + History (Spec §14).
pub fn compile_and_save(
    cfg: &AppConfig,
    intent: &str,
    persist: bool,
    cb: Option<Box<dyn FnMut(StageEvent)>>,
) -> Result<CompileReport> {
    let engine = build_engine(cfg)?;
    let outcome = engine.compile(intent, cb)?;
    let saved = if persist {
        Some(save_outcome(cfg, &outcome)?)
    } else {
        None
    };
    Ok(CompileReport { outcome, saved })
}

/// Normalisiert die CLI-Optimizer-Auswahl (`--optimizer`) auf kanonische
/// Strategie-Namen. Unbekannte Werte sind ein Konfigurationsfehler.
pub fn normalize_optimizer(s: &str) -> std::result::Result<&'static str, pf_core::error::PfError> {
    match s.to_ascii_lowercase().as_str() {
        "auto" => Ok("auto"),
        "baseline" => Ok("baseline"),
        "redundancy" => Ok("redundancy"),
        "instruction" => Ok("instruction"),
        "structural" => Ok("structural"),
        "semantic" => Ok("semantic"),
        "combined" => Ok("combined"),
        _ => Err(pf_core::error::PfError::new(
            pf_core::ErrorKind::InvalidInput,
            format!(
                "Ungültiger Optimizer '{s}' (erwartet: auto | baseline | redundancy | \
                 instruction | structural | semantic | combined)"
            ),
        )),
    }
}

/// Artefakte + History-Eintrag schreiben.
pub fn save_outcome(cfg: &AppConfig, outcome: &CompilationResult) -> Result<SaveResult> {
    let layout = HomeLayout::from_root(&cfg.home);
    layout.ensure()?;
    let ir_json = outcome.prompt_ir.to_json()?;
    let ir_path = pf_core::persist::save_artifact(&layout.generated_dir, "ir", "json", &ir_json)?;
    let long_path = pf_core::persist::save_artifact(
        &layout.generated_dir,
        "long",
        "md",
        &outcome.expanded_prompt,
    )?;
    let optimized_path = pf_core::persist::save_artifact(
        &layout.optimized_dir,
        "optimized",
        "md",
        &outcome.optimized_prompt,
    )?;

    let entry = pf_core::persist::HistoryEntry {
        request_id: outcome.request_id.clone(),
        created_at: pf_core::path::rfc3339_now(),
        intent: Some(outcome.input.clone()),
        model: cfg.llm.model.clone(),
        token_report: serde_json::to_value(&outcome.token_report).ok(),
        stages: outcome.stages.clone(),
        status: match outcome.verification.verdict {
            Some(Verdict::Pass) => "ok".to_string(),
            _ => "verify_failed".to_string(),
        },
        ir_path: Some(ir_path.display().to_string()),
        long_prompt_path: Some(long_path.display().to_string()),
        optimized_prompt_path: Some(optimized_path.display().to_string()),
        verification: serde_json::to_value(&outcome.verification).ok(),
        error: None,
    };
    let history_path = pf_core::persist::append_history(&layout.history_dir, &entry)?;
    Ok(SaveResult {
        ir_path,
        long_prompt_path: long_path,
        optimized_path,
        history_path,
    })
}

/// Kompakte menschlesbare Zusammenfassung (Spec §8/§10).
pub fn format_summary(outcome: &CompilationResult) -> String {
    let t = &outcome.token_report;
    let mut s = String::new();
    s.push_str(&format!(
        "\nPromptForge — {}\n\n",
        if outcome.llm_used {
            "Kompilierung (LLM)"
        } else {
            "Kompilierung (deterministisch, ohne LLM)"
        }
    ));
    for stage in &outcome.stages {
        s.push_str(&format!("  {stage:<12} ✓\n"));
    }
    let est = if t.estimate { " (Schätzung)" } else { "" };
    s.push_str(&format!(
        "\n  Original:   {:>6} tokens\n  Generated:  {:>6} tokens\n  Optimized:  {:>6} tokens{est}\n",
        t.original, t.generated, t.optimized
    ));
    if t.generated > 0 && t.optimized < t.generated {
        s.push_str(&format!("  Reduction:  {:>5.1}%\n", t.reduction_pct()));
    }
    let v = &outcome.verification;
    s.push_str(&format!(
        "\n  Semantic preservation: {:.2}\n  Constraints preserved:  {}\n  Output contract preserved: {}\n  Objective preserved:     {}\n  Instructions preserved:  {}\n  Verification:            {}\n",
        v.semantic_preservation,
        yesno(v.constraints_preserved),
        yesno(v.output_contract_preserved),
        yesno(v.objective_preserved),
        yesno(v.instructions_preserved),
        match v.verdict {
            Some(Verdict::Pass) => "PASS",
            _ => "FAIL",
        }
    ));
    for d in &v.details {
        s.push_str(&format!("  ({d})\n"));
    }
    // v0.2: Qualitätsmetriken sichtbar machen (Semantik + Token-Effizienz).
    let m = &outcome.metrics;
    s.push_str(&format!(
        "\n  Metrics: semantic {:.2} · structural {} · token-efficiency {:.3}\n",
        m.semantic_fidelity,
        if m.structural_validity { "ok" } else { "FAIL" },
        m.token_efficiency
    ));
    s
}

fn yesno(b: bool) -> &'static str {
    if b { "ja" } else { "nein" }
}

/// JSON-Darstellung des kompletten Ergebnisses (für `--json`).
/// v0.2-Envelope (kanonische Felder + v0.1-Aliase `ir`/`long_prompt`/
/// `final_output`) plus optionale Speicherpfade.
pub fn report_json(outcome: &CompilationResult, saved: Option<&SaveResult>) -> serde_json::Value {
    let mut v = outcome.envelope_json();
    if let Some(s) = saved {
        v["saved"] = serde_json::json!({
            "ir": s.ir_path.display().to_string(),
            "long_prompt": s.long_prompt_path.display().to_string(),
            "optimized": s.optimized_path.display().to_string(),
            "history": s.history_path.display().to_string(),
        });
    }
    v
}

/// Intent-Quelle auflösen: Argument > Datei > stdin (wenn nicht tty).
pub fn resolve_intent(
    text: Option<String>,
    file: Option<PathBuf>,
    stdin: Option<String>,
) -> Result<String> {
    if let Some(t) = text {
        return Ok(t);
    }
    if let Some(f) = file {
        let content = std::fs::read_to_string(&f).map_err(|e| {
            err(
                ErrorKind::Persistence,
                format!("{} lesen: {e}", f.display()),
            )
        })?;
        return Ok(content);
    }
    if let Some(s) = stdin {
        return Ok(s);
    }
    Err(err(
        ErrorKind::InvalidInput,
        "Kein Intent: TEXT, DATEI oder stdin angeben",
    ))
}

/// Kopiert Text in die Zwischenablage (plattformgerecht abstrahiert).
pub fn copy_optimized(outcome: &CompilationResult) -> Result<()> {
    pf_core::clipboard::copy_text(&outcome.optimized_prompt)
}

/// Liest stdin vollständig (wenn nicht tty).
pub fn read_stdin_if_piped() -> Option<String> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return None;
    }
    read_stdin_fully()
}

/// Liest stdin vollständig (auch bei tty) — für explizites `compile -`.
pub fn read_stdin_blocking() -> Result<String> {
    let t = read_stdin_fully().unwrap_or_default();
    if t.trim().is_empty() {
        return Err(err(
            ErrorKind::InvalidInput,
            "Kein Intent auf stdin (compile - erwartet Text auf stdin)",
        ));
    }
    Ok(t)
}

fn read_stdin_fully() -> Option<String> {
    let mut buf = String::new();
    use std::io::Read;
    if std::io::stdin().read_to_string(&mut buf).is_ok() {
        let t = buf.trim().to_string();
        return if t.is_empty() { None } else { Some(t) };
    }
    None
}

pub fn stage_label(stage: Stage) -> &'static str {
    stage.as_str()
}

/// Für Tests: Intent-Kompilierung mit Provider-Mock über die Python-Bridge.
pub fn engine_for_test_mock(cfg: &AppConfig) -> Result<Engine> {
    let mut c = cfg.clone();
    c.provider = ProviderKind::Mock;
    if c.llm.model.is_none() {
        c.llm.model = Some("mock-model".to_string());
    }
    for p in &c.python_paths {
        pf_bridge::push_extra_path(p.clone());
    }
    Ok(Engine::new(
        Box::new(pf_bridge::PythonBridge),
        Arc::new(HeuristicTokenizer),
        engine_config(&c),
        ProviderKind::Mock,
    ))
}

/// Kleine Hilfen für Tests.
pub fn basic_ir_from_outcome(outcome: &CompilationResult) -> &PromptIr {
    &outcome.prompt_ir
}
