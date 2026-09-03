//! pf-tui — interaktive TUI (ratatui). Bibliothek; aufgerufen vom CLI
//! (`prompt-forge tui`). Nutzt dieselbe Engine wie CLI/Service.

pub mod app;

pub use app::run_tui;
