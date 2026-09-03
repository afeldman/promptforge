//! pf-core — PromptForge-Core: Prompt IR, Config, Fehler, Token-Accounting,
//! Persistenz, Pfad-Layout, Redaction, Logging und der abstrakte
//! LLM-Bridge-Vertrag. Enthält keine LLM-/Python-Abhängigkeiten.

pub mod bridge;
pub mod clipboard;
pub mod config;
pub mod error;
pub mod ir;
pub mod log;
pub mod path;
pub mod persist;
pub mod redact;
pub mod token;

pub use bridge::{LlmBridge, LlmOperation, LlmRequest, LlmResponse, Usage};
pub use config::{
    AppConfig, DEFAULT_CONFIG_TOML, FileConfig, LlmConfig, LogConfig, ProviderKind, ServiceConfig,
    VerifyConfig,
};
pub use error::{ErrorKind, PfError, Result};
pub use ir::{
    Constraint, ConstraintSeverity, Example, IR_SCHEMA_VERSION, InputSpec, IrMetadata,
    OutputContract, PromptIr,
};
pub use token::{HeuristicTokenizer, StageTokens, TokenReport, Tokenizer};

/// Engine-Version (aus Cargo-Paketversion).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
