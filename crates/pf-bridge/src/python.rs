//! Eingebettetes Python (PyO3) für die LLM-Bridge.
//!
//! Initialisierung:
//! - `PF_PYTHON_PATH` (':' getrennt) — explizite Importpfade,
//! - sonst automatische Suche nach `python/src` / `python/.venv` relativ zum
//!   aktuellen Binary (Dev-Layout des Repos),
//! - zusätzliche Pfade können vor dem ersten Aufruf per `push_extra_path`
//!   registriert werden (z. B. aus der App-Config).
//!
//! Das Python-Paket `promptforge` muss importierbar sein; die Funktion
//! `promptforge.bridge.handle_request(request: str) -> str` ist der
//! JSON-Vertrag über die Sprachgrenze.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use pf_core::bridge::{LlmBridge, LlmRequest, LlmResponse, Usage};
use pf_core::error::{ErrorKind, Result, err};
use pyo3::prelude::*;

static EXTRA_PATHS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
static INIT: OnceLock<std::result::Result<(), String>> = OnceLock::new();

/// Registriert zusätzliche Python-Importpfade (vor dem ersten Aufruf).
pub fn push_extra_path(p: PathBuf) {
    let m = EXTRA_PATHS.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut guard) = m.lock() {
        guard.push(p);
    }
}

/// Explizite Kandidaten aus der Umgebung (`PF_PYTHON_PATH`, ':'-separiert).
fn env_candidates() -> Vec<PathBuf> {
    std::env::var("PF_PYTHON_PATH")
        .map(|v| {
            v.split(':')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Kandidaten relativ zum aktuellen Binary (Repo-Dev-Layout).
fn walkup_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return out,
    };
    let mut dir = exe.parent().and_then(Path::parent).map(Path::to_path_buf);
    for _ in 0..8 {
        let Some(d) = dir else { break };
        let src = d.join("python/src");
        let venv = d.join("python/.venv");
        if src.is_dir() {
            out.push(src);
        }
        if venv.is_dir() {
            let lib = venv.join("lib");
            if let Ok(entries) = std::fs::read_dir(&lib) {
                let mut sps: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path().join("site-packages"))
                    .filter(|p| p.is_dir())
                    .collect();
                sps.sort();
                out.extend(sps);
            }
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    out
}

fn extra_candidates() -> Vec<PathBuf> {
    EXTRA_PATHS
        .get()
        .and_then(|m| m.lock().ok())
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// Alle Import-Kandidaten, dedupliziert, nur existierende Pfade.
fn candidates() -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for p in env_candidates()
        .into_iter()
        .chain(extra_candidates())
        .chain(walkup_candidates())
    {
        if p.is_dir() && seen.insert(p.clone()) {
            out.push(p);
        }
    }
    out
}

/// Initialisiert das eingebettete Python (idempotent) und trägt die
/// Importpfade in `sys.path` ein.
pub fn ensure_initialized() -> Result<()> {
    let res = INIT.get_or_init(init_inner);
    res.clone().map_err(|msg| err(ErrorKind::Bridge, msg))
}

fn init_inner() -> std::result::Result<(), String> {
    let paths = candidates();
    Python::attach(|py| -> PyResult<()> {
        let sys = py.import("sys")?;
        let path = sys.getattr("path")?;
        for p in &paths {
            path.call_method1("insert", (0, p.to_string_lossy().to_string()))?;
        }
        Ok(())
    })
    .map_err(|e| {
        if paths.is_empty() {
            format!(
                "Keine Python-Importpfade gefunden ({e}). Setze PF_PYTHON_PATH auf das promptforge-Paket \
                 (z. B. <repo>/python/src) oder richte das uv-Venv ein (siehe README)."
            )
        } else {
            format!("Python-Init fehlgeschlagen: {e}")
        }
    })
}

/// Ruft `promptforge.bridge.handle_request(request_json)` auf.
/// Gibt die JSON-Antwort der Python-Schicht zurück.
pub fn call_json(request_json: &str) -> Result<String> {
    ensure_initialized()?;
    Python::attach(|py| -> PyResult<String> {
        let module = py.import("promptforge.bridge").map_err(|e| {
            pyo3::exceptions::PyImportError::new_err(format!(
                "promptforge nicht importierbar ({e}). Hinweis: PF_PYTHON_PATH setzen oder \
                 `cd python && uv sync` ausführen (siehe README)."
            ))
        })?;
        let res = module.call_method1("handle_request", (request_json,))?;
        res.extract::<String>()
    })
    .map_err(|e| {
        let msg = format!("Python-Bridge-Fehler: {e}");
        tracing::warn!("{msg}");
        err(ErrorKind::Bridge, msg)
    })
}

/// Bridge-Implementierung über eingebettetes Python.
#[derive(Debug, Clone, Default)]
pub struct PythonBridge;

impl LlmBridge for PythonBridge {
    fn complete(&self, request: &LlmRequest) -> Result<LlmResponse> {
        let request_json = serde_json::to_string(request)?;
        let started = std::time::Instant::now();
        let response_json = call_json(&request_json)?;
        let duration_ms = started.elapsed().as_millis() as u64;

        let value: serde_json::Value = serde_json::from_str(&response_json)?;
        if let Some(err_obj) = value.get("error") {
            let kind = err_obj
                .get("kind")
                .and_then(|k| k.as_str())
                .and_then(kind_from_str)
                .unwrap_or(ErrorKind::Bridge);
            let message = err_obj
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unbekannter Python-Fehler")
                .to_string();
            return Err(err(kind, format!("Python: {message}")));
        }
        let content = value
            .get("content")
            .and_then(|c| c.as_str())
            .ok_or_else(|| err(ErrorKind::Bridge, "Python-Antwort ohne 'content'"))?
            .to_string();
        let usage = value.get("usage").map(|u| {
            let pt = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            let ct = u
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Usage {
                prompt_tokens: pt,
                completion_tokens: ct,
                total_tokens: pt + ct,
            }
        });
        Ok(LlmResponse {
            content,
            finish_reason: value
                .get("finish_reason")
                .and_then(|f| f.as_str())
                .map(str::to_string),
            usage,
            model: value
                .get("model")
                .and_then(|m| m.as_str())
                .map(str::to_string),
            duration_ms: Some(duration_ms),
        })
    }
}

fn kind_from_str(s: &str) -> Option<ErrorKind> {
    match s {
        "configuration" => Some(ErrorKind::Configuration),
        "provider" => Some(ErrorKind::Provider),
        "authentication" => Some(ErrorKind::Authentication),
        "model" => Some(ErrorKind::Model),
        "timeout" => Some(ErrorKind::Timeout),
        "tokenization" => Some(ErrorKind::Tokenization),
        "optimization" => Some(ErrorKind::Optimization),
        "verification" => Some(ErrorKind::Verification),
        "persistence" => Some(ErrorKind::Persistence),
        "bridge" => Some(ErrorKind::Bridge),
        "invalid_input" => Some(ErrorKind::InvalidInput),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_mapping() {
        assert_eq!(kind_from_str("timeout"), Some(ErrorKind::Timeout));
        assert_eq!(kind_from_str("nope"), None);
    }
}
