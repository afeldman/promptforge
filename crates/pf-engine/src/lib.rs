//! pf-engine — Pipeline-Orchestrierung, Expansion, Optimizer-Passes,
//! Verifikation und MockBridge. Kennt keine Python-Details; LLM-Zugriff
//! ausschließlich über `pf_core::bridge::LlmBridge`.

pub mod expand;
pub mod mock;
pub mod optimizer;
pub mod pipeline;
pub mod verify;

pub use optimizer::OptimizerEvent;
pub use pf_core::compilation::{CompilationResult, QualityMetrics};
pub use pipeline::{Engine, EngineConfig, Stage, StageEvent};
pub use verify::{SemanticReport, Verdict, VerificationReport};

use pf_core::config::VerifyConfig;
use pf_core::error::Result;

impl From<VerifyConfig> for EngineConfig {
    fn from(v: VerifyConfig) -> Self {
        EngineConfig {
            llm: Default::default(),
            verify: v,
        }
    }
}

/// Führt eine Kompilierung deterministisch (ohne LLM) aus — für Tests/Tools.
pub fn compile_deterministic(intent: &str) -> Result<CompilationResult> {
    Engine::deterministic(EngineConfig::from(VerifyConfig::default())).compile(intent, None)
}
