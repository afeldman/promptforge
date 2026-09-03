//! TUI-Anwendung (ratatui): Input, Pipeline-Status, Token-Zahlen,
//! Verifikation, Prompt-Vorschau, Copy/Save-Aktionen (Spec §11).

use std::io;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use pf_core::config::AppConfig;
use pf_core::error::{ErrorKind, Result, err};
use pf_core::token::TokenReport;
use pf_engine::{
    CompilationResult, Engine, QualityMetrics, StageEvent, Verdict, VerificationReport,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

/// UI-Zustand.
struct TuiApp {
    input: String,
    running: bool,
    /// Liste der abgeschlossenen Stadien (name, ok).
    stages: Vec<(String, bool)>,
    token_report: Option<TokenReport>,
    verification: Option<VerificationReport>,
    long_prompt: Option<String>,
    optimized_prompt: Option<String>,
    show_optimized: bool,
    status: String,
    error: Option<String>,
    copied: bool,
}

impl TuiApp {
    fn new() -> Self {
        Self {
            input: String::new(),
            running: false,
            stages: Vec::new(),
            token_report: None,
            verification: None,
            long_prompt: None,
            optimized_prompt: None,
            show_optimized: true,
            status: "Bereit. Intent eingeben, Enter kompiliert.".to_string(),
            error: None,
            copied: false,
        }
    }

    fn clear_result(&mut self) {
        self.stages.clear();
        self.token_report = None;
        self.verification = None;
        self.long_prompt = None;
        self.optimized_prompt = None;
        self.copied = false;
    }
}

/// Startet die TUI (blockierend).
pub fn run_tui(cfg: &AppConfig, engine: Arc<Engine>) -> Result<()> {
    enable_raw_mode().map_err(|e| err(ErrorKind::Io, e.to_string()))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|e| err(ErrorKind::Io, e.to_string()))?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| err(ErrorKind::Io, e.to_string()))?;

    let result = event_loop(cfg, engine, &mut terminal);

    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    result
}

enum UiMsg {
    Stage(StageEvent),
    Done(Box<std::result::Result<CompilationResult, pf_core::PfError>>),
}

#[allow(clippy::collapsible_if)] // Event-Loop-Struktur bleibt lesbar
fn event_loop(
    cfg: &AppConfig,
    engine: Arc<Engine>,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<()> {
    let mut app = TuiApp::new();
    let (tx, rx): (Sender<UiMsg>, Receiver<UiMsg>) = std::sync::mpsc::channel();

    loop {
        terminal.draw(|f| draw(f, &app))?;

        if event::poll(std::time::Duration::from_millis(100))
            .map_err(|e| err(ErrorKind::Io, e.to_string()))?
        {
            if let Event::Key(key) = event::read().map_err(|e| err(ErrorKind::Io, e.to_string()))? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') if app.input.is_empty() => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    KeyCode::Char('p') if !app.running => {
                        app.show_optimized = !app.show_optimized;
                    }
                    KeyCode::Char('y') if !app.running => {
                        app.copied = false;
                        let outcome_prompt = if app.show_optimized {
                            app.optimized_prompt.clone()
                        } else {
                            app.long_prompt.clone()
                        };
                        if let Some(text) = outcome_prompt {
                            match pf_core::clipboard::copy_text(&text) {
                                Ok(()) => {
                                    app.copied = true;
                                    app.status = "In Zwischenablage kopiert (y).".to_string();
                                }
                                Err(e) => app.status = format!("Kopieren fehlgeschlagen: {e}"),
                            }
                        }
                    }
                    KeyCode::Char('s') if !app.running => {
                        if let Some(outcome) = current_outcome(&app) {
                            match pf_cli_save(cfg, &outcome) {
                                Ok(paths) => app.status = format!("Gespeichert nach {}", paths),
                                Err(e) => app.status = format!("Speichern fehlgeschlagen: {e}"),
                            }
                        }
                    }
                    KeyCode::Char(c) if !app.running && !app.input.ends_with('\n') => {
                        app.input.push(c);
                        app.status = "Bereit. Enter kompiliert.".to_string();
                    }
                    KeyCode::Backspace if !app.running => {
                        app.input.pop();
                    }
                    KeyCode::Enter if !app.running && !app.input.trim().is_empty() => {
                        app.clear_result();
                        app.running = true;
                        app.status = "Pipeline läuft …".to_string();
                        let intent = app.input.trim().to_string();
                        let tx2 = tx.clone();
                        let engine2 = Arc::clone(&engine);
                        let tx_done = tx2.clone();
                        std::thread::spawn(move || {
                            let tx_ev = tx2.clone();
                            let cb: Box<dyn FnMut(StageEvent)> = Box::new(move |ev| {
                                let _ = tx_ev.send(UiMsg::Stage(ev));
                            });
                            let result = engine2.compile(&intent, Some(cb));
                            let _ = tx_done.send(UiMsg::Done(Box::new(result)));
                        });
                    }
                    _ => {}
                }
            }
        }

        while let Ok(msg) = rx.try_recv() {
            match msg {
                UiMsg::Stage(ev) => match ev {
                    StageEvent::StageStarted(stage) => {
                        app.status = format!("Stufe läuft: {} …", stage.as_str());
                    }
                    StageEvent::StageFinished { stage, ok } => {
                        app.stages.push((stage.as_str().to_string(), ok));
                    }
                    StageEvent::LlmUsage { stage, usage } => {
                        app.status = format!(
                            "LLM-Call ({}) prompt={} completion={}",
                            stage.as_str(),
                            usage.prompt_tokens,
                            usage.completion_tokens
                        );
                    }
                    StageEvent::Note(note) => {
                        app.status = note;
                    }
                    // Debug-Trace-Events (--debug/--debug-json) interessieren
                    // die TUI nicht; hier nur abfangen.
                    StageEvent::LlmTrace { .. } => {}
                },
                UiMsg::Done(boxed) => match *boxed {
                    Ok(outcome) => {
                        app.token_report = Some(outcome.token_report.clone());
                        app.verification = Some(outcome.verification.clone());
                        app.long_prompt = Some(outcome.expanded_prompt.clone());
                        app.optimized_prompt = Some(outcome.optimized_prompt.clone());
                        app.running = false;
                        app.status = match outcome.verification.verdict {
                            Some(Verdict::Pass) => {
                                "Verifikation: PASS — [p] Vorschau, [y] Copy, [s] Save, [q] Quit"
                                    .to_string()
                            }
                            _ => "Verifikation: FAIL — Details unten".to_string(),
                        };
                    }
                    Err(e) => {
                        app.running = false;
                        app.error = Some(e.to_string());
                        app.status = format!("Fehler: {e}");
                    }
                },
            }
        }
        let _ = cfg;
    }
}

fn current_outcome(app: &TuiApp) -> Option<CompilationResult> {
    let token_report = app.token_report.clone()?;
    let verification = app.verification.clone()?;
    let metrics = QualityMetrics::compute(&verification, &token_report);
    Some(CompilationResult {
        input: app.input.trim().to_string(),
        request_id: "tui".to_string(),
        llm_used: true,
        stages: app.stages.iter().map(|(s, _)| s.clone()).collect(),
        prompt_ir: pf_core::ir::PromptIr::new("tui", ""),
        expanded_prompt: app.long_prompt.clone()?,
        optimized_prompt: app.optimized_prompt.clone()?,
        token_report,
        verification,
        metrics,
        optimization: None,
    })
}

/// Speichert Artefakte des letzten Laufs (kleiner Duplikat-Anteil zu pf-cli,
/// um die TUI unabhängig vom CLI-Crate zu halten).
fn pf_cli_save(cfg: &AppConfig, outcome: &CompilationResult) -> Result<String> {
    let layout = pf_core::path::HomeLayout::from_root(&cfg.home);
    layout.ensure()?;
    let ir_json = outcome.prompt_ir.to_json()?;
    let ir_path = pf_core::persist::save_artifact(&layout.generated_dir, "ir", "json", &ir_json)?;
    let long_path = pf_core::persist::save_artifact(
        &layout.generated_dir,
        "long",
        "md",
        &outcome.expanded_prompt,
    )?;
    let opt_path = pf_core::persist::save_artifact(
        &layout.optimized_dir,
        "optimized",
        "md",
        &outcome.optimized_prompt,
    )?;
    let entry = pf_core::persist::HistoryEntry {
        request_id: outcome.request_id.clone(),
        created_at: pf_core::path::rfc3339_now(),
        intent: Some(outcome.input.clone()),
        model: None,
        token_report: serde_json::to_value(&outcome.token_report).ok(),
        stages: outcome.stages.clone(),
        status: "ok".to_string(),
        ir_path: Some(ir_path.display().to_string()),
        long_prompt_path: Some(long_path.display().to_string()),
        optimized_prompt_path: Some(opt_path.display().to_string()),
        verification: None,
        error: None,
    };
    let _history = pf_core::persist::append_history(&layout.history_dir, &entry)?;
    Ok(format!(
        "{} + {} + {} (+ History)",
        ir_path.display(),
        long_path.display(),
        opt_path.display()
    ))
}

fn draw(f: &mut ratatui::Frame, app: &TuiApp) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(14),
            Constraint::Min(6),
        ])
        .split(area);
    draw_input(f, app, chunks[0]);
    draw_status(f, app, chunks[1]);
    draw_preview(f, app, chunks[2]);
}

fn draw_input(f: &mut ratatui::Frame, app: &TuiApp, area: Rect) {
    let title = if app.running {
        "PromptForge — Pipeline läuft (Enter gesperrt)"
    } else {
        "PromptForge — Intent (Enter = kompilieren, q = quit)"
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let text = if app.input.is_empty() {
        Line::from(Span::styled(
            "z. B. Analysiere diese fünf Papers, vergleiche die Methoden …",
            Style::default().fg(Color::DarkGray),
        ))
    } else {
        Line::from(app.input.clone())
    };
    let p = Paragraph::new(text).block(block);
    f.render_widget(p, area);
}

fn draw_status(f: &mut ratatui::Frame, app: &TuiApp, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();
    for (stage, ok) in &app.stages {
        let mark = if *ok { "✓" } else { "✗" };
        items.push(ListItem::new(Line::from(vec![Span::raw(format!(
            "  {stage:<12} {mark}"
        ))])));
    }
    if let Some(t) = &app.token_report {
        items.push(ListItem::new(Line::from(Span::styled(
            format!(
                "  Tokens: original {} → generated {} → optimized {} (Reduktion {:.1}%)",
                t.original,
                t.generated,
                t.optimized,
                t.reduction_pct()
            ),
            Style::default().fg(Color::Cyan),
        ))));
    }
    if let Some(v) = &app.verification {
        let verdict = match v.verdict {
            Some(Verdict::Pass) => "PASS",
            _ => "FAIL",
        };
        let color = if v.verdict == Some(Verdict::Pass) {
            Color::Green
        } else {
            Color::Red
        };
        items.push(ListItem::new(Line::from(Span::styled(
            format!(
                "  Verifikation: {verdict} | semantic {:.2} | constraints {} | contract {} | objective {}",
                v.semantic_preservation,
                yes(v.constraints_preserved),
                yes(v.output_contract_preserved),
                yes(v.objective_preserved)
            ),
            Style::default().fg(color),
        ))));
    }
    if let Some(e) = &app.error {
        items.push(ListItem::new(Line::from(Span::styled(
            format!("  Fehler: {e}"),
            Style::default().fg(Color::Red),
        ))));
    }
    items.push(ListItem::new(Line::from(Span::styled(
        format!(
            "  {}  |  [p] Long/Optimiert  [y] Copy  [s] Save  [q] Quit",
            app.status
        ),
        Style::default().fg(Color::DarkGray),
    ))));
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Pipeline / Ergebnis"),
    );
    f.render_widget(list, area);
}

fn draw_preview(f: &mut ratatui::Frame, app: &TuiApp, area: Rect) {
    let (title, text) = if app.show_optimized {
        (
            "Prompt-Vorschau (optimiert)",
            app.optimized_prompt.clone().unwrap_or_default(),
        )
    } else {
        (
            "Prompt-Vorschau (Long Prompt)",
            app.long_prompt.clone().unwrap_or_default(),
        )
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(Line::from(Span::styled(
            "Scroll: Mausrad",
            Style::default().fg(Color::DarkGray),
        )));
    let p = Paragraph::new(text).block(block).scroll((0, 0));
    f.render_widget(p, area);
}

fn yes(b: bool) -> &'static str {
    if b { "ja" } else { "nein" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_tracks_stages_and_result() {
        let mut app = TuiApp::new();
        app.input.push_str("Testintent");
        assert_eq!(app.input, "Testintent");
        app.stages.push(("architect".to_string(), true));
        assert_eq!(app.stages.len(), 1);
    }

    #[test]
    fn yesno_formats() {
        assert_eq!(yes(true), "ja");
        assert_eq!(yes(false), "nein");
    }
}
