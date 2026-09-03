//! pf-core — PromptForge-Core: Prompt IR, Config, Fehler, Token-Accounting,
//! Persistenz, Pfad-Layout, Redaction, Logging und der abstrakte
//! LLM-Bridge-Vertrag. Enthält keine LLM-/Python-Abhängigkeiten.

pub mod bridge;
pub mod clipboard;
pub mod compilation;
pub mod config;
pub mod error;
pub mod ir;
pub mod log;
pub mod path;
pub mod persist;
pub mod redact;
pub mod serialize;
pub mod token;
pub mod verify;

pub use bridge::{LlmBridge, LlmOperation, LlmRequest, LlmResponse, Usage};
pub use compilation::{
    CandidateReport, CompilationResult, OptimizationReport, OptimizationStatus, QualityMetrics,
};
pub use config::{
    AppConfig, DEFAULT_CONFIG_TOML, FileConfig, LlmConfig, LogConfig, ProviderKind, ServiceConfig,
    VerifyConfig,
};
pub use error::{ErrorKind, PfError, Result};
pub use ir::{
    Constraint, ConstraintSeverity, Example, IR_SCHEMA_VERSION, InputSpec, IntentAnalysis,
    IrMetadata, OutputContract, PromptIr,
};
pub use serialize::{
    JsonSerializer, OutputFormat, PromptSerializer, TextSerializer, ToonSerializer, YamlSerializer,
    render_structured, serializer_for, to_json_string, to_toon_string, to_yaml_string,
    toon_to_json, yaml_to_json,
};
pub use token::{HeuristicTokenizer, StageTokens, TokenReport, Tokenizer};
pub use verify::{CheckResult, SemanticReport, Verdict, VerificationReport};

/// Engine-Version (aus Cargo-Paketversion).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
