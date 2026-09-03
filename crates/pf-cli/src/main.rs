//! `prompt-forge` — CLI-Binary (Spec §9/§10/§22).
//!
//! Subcommands: init | compile [TEXT|DATEI] | serve | tui.
//! Scriptbar: ohne TTY wird der optimierte Prompt auf stdout geschrieben,
//! Statistiken auf stderr; `--json` liefert alles maschinenlesbar.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use pf_core::config::{AppConfig, ProviderKind};
use pf_core::error::Result;
use pf_engine::StageEvent;

#[derive(Parser)]
#[command(
    name = "prompt-forge",
    version = pf_core::VERSION,
    about = "Local-first Prompt Compiler: Intent → IR → Expansion → Optimierung → Verifikation"
)]
struct Cli {
    /// User-Home (Default: PF_HOME bzw. ~/.prompt-forge)
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Legt User-Home + Default-Konfiguration an
    Init,
    /// Kompiliert einen Intent zu einem optimierten Prompt
    Compile(CompileArgs),
    /// Startet den lokalen HTTP-Service
    Serve,
    /// Startet die interaktive TUI
    Tui,
}

#[derive(clap::Args)]
struct CompileArgs {
    /// Intent als Text (alternativ DATEI oder stdin)
    intent: Option<String>,
    /// Intent aus Datei lesen
    #[arg(short = 'f', long = "file")]
    file: Option<PathBuf>,
    /// Optimierten Prompt in Datei schreiben
    #[arg(short = 'o', long = "out")]
    out: Option<PathBuf>,
    /// Prompt in die Zwischenablage kopieren
    #[arg(long)]
    copy: bool,
    /// Komplettes Ergebnis als JSON auf stdout
    #[arg(long)]
    json: bool,
    /// Nur den optimierten Prompt auf stdout (kein Menü)
    #[arg(short = 'p', long = "plain")]
    plain: bool,
    /// Artefakte + History speichern
    #[arg(long)]
    save: bool,
    /// Provider-Override: auto | any_llm | mock | none
    #[arg(long)]
    provider: Option<String>,
    /// LLM_ENDPOINT-Override
    #[arg(long)]
    endpoint: Option<String>,
    /// LLM_KEY-Override
    #[arg(long)]
    key: Option<String>,
    /// LLM_MODEL-Override
    #[arg(long)]
    model: Option<String>,
    /// Ohne LLM (deterministische Pipeline)
    #[arg(long)]
    no_llm: bool,
}

fn main() {
    let code = real_main();
    std::process::exit(code);
}

fn real_main() -> i32 {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Fehler ({}): {e}", e.kind.exit_code());
            e.kind.exit_code()
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let mut cfg = AppConfig::load(cli.home.as_deref())?;
    let layout = pf_core::path::HomeLayout::from_root(&cfg.home);
    layout.ensure()?;

    match cli.cmd {
        Cmd::Init => {
            let messages = pf_cli::app::run_init(&cfg)?;
            for m in messages {
                println!("{m}");
            }
            Ok(())
        }
        Cmd::Compile(args) => run_compile(&mut cfg, args),
        Cmd::Serve => {
            let engine = Arc::new(pf_cli::app::build_engine(&cfg)?);
            pf_service::serve(&cfg, engine)
        }
        Cmd::Tui => {
            let engine = Arc::new(pf_cli::app::build_engine(&cfg)?);
            pf_tui::run_tui(&cfg, engine)
        }
    }
}

fn run_compile(cfg: &mut AppConfig, args: CompileArgs) -> Result<()> {
    apply_overrides(cfg, &args)?;

    // Logging (Rolling-Dateien, redigiert) — nach ensure_home.
    let _guard = {
        let secrets = pf_core::log::collect_secrets(cfg);
        pf_core::log::init_logging(&cfg.log, &secrets, &layout_logs_dir(cfg))?
    };

    let stdin_text = pf_cli::app::read_stdin_if_piped();
    let intent = pf_cli::app::resolve_intent(args.intent, args.file, stdin_text)?;

    // Interaktives Menü nur bei TTY + nicht plain/json + keinem -o.
    let interactive = std::io::stdout().is_terminal()
        && std::io::stdin().is_terminal()
        && !args.plain
        && !args.json
        && args.out.is_none();

    // Nicht-interaktiv: ein Lauf, dann Ausgabe.
    if !interactive {
        let engine = pf_cli::app::build_engine(cfg)?;
        let cb = stage_logger();
        let outcome = engine.compile(&intent, Some(cb))?;
        let _ = append_meta_history(cfg, &outcome);

        if args.json {
            let saved = if args.save || args.out.is_some() {
                pf_cli::app::save_outcome(cfg, &outcome).ok()
            } else {
                None
            };
            let json = pf_cli::app::report_json(&outcome, saved.as_ref());
            println!("{}", serde_json::to_string_pretty(&json)?);
            return Ok(());
        }

        // Scriptmodus: Statistik auf stderr, Prompt auf stdout/Datei.
        eprint!("{}", pf_cli::app::format_summary(&outcome));
        if let Some(out) = &args.out {
            pf_core::persist::atomic_write(out, outcome.optimized_prompt.as_bytes())?;
            eprintln!("Geschrieben: {}", out.display());
        } else {
            println!("{}", outcome.optimized_prompt);
        }
        if args.copy {
            pf_cli::app::copy_optimized(&outcome)?;
            eprintln!("Prompt in Zwischenablage kopiert.");
        }
        return Ok(());
    }

    // Interaktiver Modus (Spec §10): Menü-Schleife; [4] kompiliert neu.
    'compile_loop: loop {
        let engine = pf_cli::app::build_engine(cfg)?;
        let outcome = engine.compile(&intent, Some(stage_logger()))?;
        let _ = append_meta_history(cfg, &outcome);
        eprint!("{}", pf_cli::app::format_summary(&outcome));
        let mut saved = false;

        loop {
            match menu_choice()? {
                MenuAction::Copy => match pf_cli::app::copy_optimized(&outcome) {
                    Ok(()) => eprintln!("Prompt in Zwischenablage kopiert."),
                    Err(e) => eprintln!("Kopieren fehlgeschlagen: {e}"),
                },
                MenuAction::Save => {
                    if saved {
                        eprintln!("Bereits gespeichert.");
                    } else {
                        match pf_cli::app::save_outcome(cfg, &outcome) {
                            Ok(s) => {
                                eprintln!("Gespeichert:");
                                eprintln!("  IR:        {}", s.ir_path.display());
                                eprintln!("  Long:      {}", s.long_prompt_path.display());
                                eprintln!("  Optimized: {}", s.optimized_path.display());
                                saved = true;
                            }
                            Err(e) => eprintln!("Speichern fehlgeschlagen: {e}"),
                        }
                    }
                }
                MenuAction::Show => println!("{}", outcome.optimized_prompt),
                MenuAction::Recompile => continue 'compile_loop,
                MenuAction::Quit => return Ok(()),
            }
        }
    }
}

/// Fortschritts-Callback für Skript-/stderr-Kontexte (LLM-Usage + Notizen).
fn stage_logger() -> Box<dyn FnMut(StageEvent)> {
    Box::new(|ev| match ev {
        StageEvent::StageStarted(stage) => {
            if !std::io::stderr().is_terminal() {
                eprintln!("[pipeline] start: {}", stage.as_str());
            }
        }
        StageEvent::StageFinished { stage, ok } => {
            if !std::io::stderr().is_terminal() {
                eprintln!(
                    "[pipeline] end: {} ({})",
                    stage.as_str(),
                    if ok { "ok" } else { "fail" }
                );
            }
        }
        StageEvent::Note(note) => {
            if !std::io::stderr().is_terminal() {
                eprintln!("[pipeline] note: {note}");
            }
        }
        StageEvent::LlmUsage { stage, usage } => {
            eprintln!(
                "[usage] {} prompt={} completion={}",
                stage.as_str(),
                usage.prompt_tokens,
                usage.completion_tokens
            );
        }
    })
}

enum MenuAction {
    Copy,
    Save,
    Show,
    Recompile,
    Quit,
}

fn menu_choice() -> Result<MenuAction> {
    println!("\n[1] Copy optimized prompt");
    println!("[2] Save prompt");
    println!("[3] Show prompt");
    println!("[4] Recompile");
    println!("[q] Quit");
    print!("> ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| pf_core::error::PfError::new(pf_core::ErrorKind::Io, e.to_string()))?;
    match line.trim().to_ascii_lowercase().as_str() {
        "1" => Ok(MenuAction::Copy),
        "2" => Ok(MenuAction::Save),
        "3" => Ok(MenuAction::Show),
        "4" | "r" => Ok(MenuAction::Recompile),
        "q" | "quit" => Ok(MenuAction::Quit),
        _ => {
            eprintln!("Unbekannte Eingabe.");
            menu_choice()
        }
    }
}

fn apply_overrides(cfg: &mut AppConfig, args: &CompileArgs) -> Result<()> {
    if let Some(p) = &args.provider {
        let kind = ProviderKind::parse(p).ok_or_else(|| {
            pf_core::error::PfError::new(
                pf_core::ErrorKind::Configuration,
                format!("unbekannter Provider: {p}"),
            )
        })?;
        cfg.provider = kind;
    }
    if args.no_llm {
        cfg.provider = ProviderKind::None;
    }
    if let Some(v) = &args.endpoint {
        cfg.llm.endpoint = Some(v.clone());
    }
    if let Some(v) = &args.key {
        cfg.llm.key = Some(v.clone());
    }
    if let Some(v) = &args.model {
        cfg.llm.model = Some(v.clone());
    }
    Ok(())
}

fn layout_logs_dir(cfg: &AppConfig) -> PathBuf {
    cfg.home.join("logs")
}

fn append_meta_history(cfg: &AppConfig, outcome: &pf_engine::CompileOutcome) -> Result<()> {
    let layout = pf_core::path::HomeLayout::from_root(&cfg.home);
    layout.ensure()?;
    let entry = pf_core::persist::HistoryEntry {
        request_id: outcome.request_id.clone(),
        created_at: pf_core::path::rfc3339_now(),
        intent: None,
        model: cfg.llm.model.clone(),
        token_report: serde_json::to_value(&outcome.token_report).ok(),
        stages: outcome.stages_done.clone(),
        status: match outcome.verification.verdict {
            Some(pf_engine::Verdict::Pass) => "ok".to_string(),
            _ => "verify_failed".to_string(),
        },
        ir_path: None,
        long_prompt_path: None,
        optimized_prompt_path: None,
        verification: serde_json::to_value(&outcome.verification).ok(),
        error: None,
    };
    pf_core::persist::append_history(&layout.history_dir, &entry)?;
    Ok(())
}
