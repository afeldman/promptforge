//! pf-bridge — PyO3-Boundary: eingebettetes CPython als LLM-Bridge.
//!
//! Rust → JSON-Request → Python (`promptforge.bridge.handle_request`) →
//! LLM (any-llm oder Mock) → strukturierte JSON-Antwort → Rust.
//! Ein Aufruf pro LLM-Operation (transaktional, Spec §1/§2).

pub mod python;

pub use python::{PythonBridge, ensure_initialized, push_extra_path};

/// Version der Python-Bridge (für Diagnose).
pub fn bridge_version() -> String {
    format!("pf-bridge {}", env!("CARGO_PKG_VERSION"))
}
