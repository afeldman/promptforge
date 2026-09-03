//! Logging-Initialisierung (Spec §16/§17).
//!
//! Rust: `tracing` + `tracing-subscriber`/`tracing-appender`, Rolling-Dateien
//! `~/.prompt-forge/logs/prompt-forge.{log,1.log,…}`, Retention konfigurierbar,
//! `format = "json"` für Service/Machine-Mode. Ein Sanitizing-Writer erzwingt
//! Secret-Redaction auf Dateiebene (zusätzlich zu redigierten Meldungen).

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

use crate::config::LogConfig;
use crate::error::{ErrorKind, Result, err};
use crate::redact::sanitize_line;

static INIT: OnceLock<()> = OnceLock::new();

/// Muss am Leben bleiben, solange Logs geschrieben werden (sonst Puffer-Verlust).
pub struct LogGuard {
    _worker: Option<WorkerGuard>,
}

fn filter_for(level: &str) -> EnvFilter {
    let normalized = match level.to_ascii_lowercase().as_str() {
        "trace" => "trace",
        "debug" => "debug",
        "warning" | "warn" => "warn",
        "error" => "error",
        _ => "info",
    };
    EnvFilter::new(normalized)
}

/// Initialisiert das Global-Logging. Idempotent: ein zweiter Aufruf ist ein
/// No-op (liefert einen leeren Guard).
pub fn init_logging(cfg: &LogConfig, secrets: &[String], logs_dir: &Path) -> Result<LogGuard> {
    if INIT.set(()).is_err() {
        return Ok(LogGuard { _worker: None });
    }

    if let Err(e) = std::fs::create_dir_all(logs_dir) {
        // Fallback: ohne Datei-Logging weiterlaufen (z. B. read-only Home).
        eprintln!(
            "WARN: Log-Verzeichnis {} nicht anlegbar ({e}) — kein Datei-Logging",
            logs_dir.display()
        );
        return Ok(LogGuard { _worker: None });
    }

    let rolling = Builder::new()
        .rotation(Rotation::HOURLY)
        .filename_prefix("prompt-forge")
        .max_log_files(cfg.retention.max(1))
        .build(logs_dir)
        .map_err(|e| err(ErrorKind::Persistence, format!("Log-Rolling init: {e}")))?;

    let (nb, guard) = tracing_appender::non_blocking(rolling);
    let factory = SanitizingFactory {
        inner: nb,
        secrets: Arc::new(secrets.to_vec()),
    };
    let filter = filter_for(&cfg.level);

    if cfg.format.eq_ignore_ascii_case("json") {
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .with_writer(factory)
            .with_ansi(false)
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .map_err(|e| err(ErrorKind::Io, format!("tracing init: {e}")))?;
    } else {
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(factory)
            .with_ansi(false)
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .map_err(|e| err(ErrorKind::Io, format!("tracing init: {e}")))?;
    }
    Ok(LogGuard {
        _worker: Some(guard),
    })
}

struct SanitizingFactory {
    inner: NonBlocking,
    secrets: Arc<Vec<String>>,
}

struct SanitizingWriter {
    inner: NonBlocking,
    secrets: Arc<Vec<String>>,
    buf: Vec<u8>,
}

impl SanitizingWriter {
    fn sanitized_line(&self, raw: &[u8]) -> Vec<u8> {
        let text = String::from_utf8_lossy(raw);
        sanitize_line(&text, &self.secrets).into_bytes()
    }

    fn flush_complete_lines(&mut self) -> std::io::Result<()> {
        let mut consumed = 0;
        while let Some(pos) = self.buf[consumed..].iter().position(|&b| b == b'\n') {
            let end = consumed + pos + 1;
            let line = self.sanitized_line(&self.buf[consumed..end]);
            self.inner.write_all(&line)?;
            consumed = end;
        }
        if consumed > 0 {
            self.buf.drain(..consumed);
        }
        Ok(())
    }
}

impl Write for SanitizingWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        self.flush_complete_lines()?;
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flush_complete_lines()?;
        if !self.buf.is_empty() {
            let rest = std::mem::take(&mut self.buf);
            self.inner.write_all(&self.sanitized_line(&rest))?;
        }
        self.inner.flush()
    }
}

impl<'a> MakeWriter<'a> for SanitizingFactory {
    type Writer = SanitizingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        SanitizingWriter {
            inner: self.inner.clone(),
            secrets: Arc::clone(&self.secrets),
            buf: Vec::new(),
        }
    }
}

/// Sammelt Secret-Werte aus Konfiguration + Environment für die Redaction.
pub fn collect_secrets(cfg: &crate::config::AppConfig) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(k) = cfg.llm.key.as_deref().filter(|k| !k.is_empty()) {
        out.push(k.to_string());
    }
    if let Some(k) = std::env::var("LLM_KEY").ok().filter(|k| !k.is_empty()) {
        out.push(k);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_levels_normalize() {
        let warn = format!("{:?}", filter_for("WARNING"));
        assert!(warn.contains("LevelFilter::WARN"), "warn={warn}");
        let info = format!("{:?}", filter_for("bogus"));
        assert!(info.contains("LevelFilter::INFO"), "info={info}");
    }
}
