//! Abstrakte LLM-Schnittstelle (Spec §2/§5).
//!
//! Der Rust-Core kennt ausschließlich diesen Vertrag. Python kapselt
//! any-llm/Provider; die Implementierung `PythonBridge` liegt in pf-bridge.
//! `api_key` existiert nur im Speicher und wird niemals geloggt (redigiertes
//! Debug).

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Pipeline-Operation, die einen LLM-Aufruf benötigt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmOperation {
    /// Intent (natürliche Sprache) → strukturierte Prompt-IR (JSON).
    Architect,
    /// Long Prompt (+ IR) → optimierter Prompt.
    Optimize,
    /// Original vs. optimiert → semantischer Verifikationsbericht.
    Verify,
    /// Beliebiger Chat-Call (z. B. `POST /v1/execute`).
    Chat,
}

impl LlmOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            LlmOperation::Architect => "architect",
            LlmOperation::Optimize => "optimize",
            LlmOperation::Verify => "verify",
            LlmOperation::Chat => "chat",
        }
    }
}

/// Tokenverbrauch eines LLM-Aufrufs (falls vom Provider geliefert).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// Ein LLM-Request über die Sprachgrenze (JSON-Vertrag).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub operation: LlmOperation,
    /// System-Prompt (Verhaltens-/Formatvorgaben der Python-Schicht).
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Nutzer-/Aufgabeninhalt (enthält ggf. den Prompt-Text).
    pub user_prompt: String,
    /// Erwartetes Ausgabe-Schema (JSON-Schema-ähnlich), optional.
    #[serde(default)]
    pub output_schema: Option<serde_json::Value>,
    // Provider-Konfiguration (durchgereicht, redigiert geloggt):
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Secret — nur im Speicher und im In-Memory-Bridge-JSON; niemals loggen.
    /// (Kein `skip_serializing`: die Python-Schicht benötigt den Wert.)
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Provider-Auswahl für die Python-Schicht: "auto" | "any_llm" | "mock".
    #[serde(default = "default_provider")]
    pub provider: String,
    pub request_id: String,
    #[serde(default)]
    pub timeout_s: Option<u64>,
}

fn default_provider() -> String {
    "auto".to_string()
}

impl std::fmt::Display for LlmRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LlmRequest(op={}, model={:?}, provider={}, req={}, user_chars={})",
            self.operation.as_str(),
            self.model,
            self.provider,
            self.request_id,
            self.user_prompt.chars().count()
        )
    }
}

/// Strukturierte LLM-Antwort über die Sprachgrenze.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

impl LlmResponse {
    /// JSON-Text aus `content` extrahieren (Codeblock-Toleranz).
    pub fn parse_content_json(&self) -> Result<serde_json::Value> {
        let text = self.content.trim();
        let stripped = strip_code_fence(text);
        serde_json::from_str(stripped).map_err(|e| {
            crate::error::PfError::new(
                crate::error::ErrorKind::Model,
                format!("LLM-Antwort ist kein valides JSON: {e}"),
            )
        })
    }
}

/// Entfernt optionale ```json … ```-Fences.
pub fn strip_code_fence(text: &str) -> &str {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("```") {
        let after_lang = rest
            .strip_prefix("json")
            .or_else(|| rest.strip_prefix("JSON"))
            .unwrap_or(rest);
        let after_lang = after_lang.strip_prefix('\n').unwrap_or(after_lang);
        if let Some(end) = after_lang.rfind("```") {
            return after_lang[..end].trim();
        }
        return after_lang.trim();
    }
    t
}

/// Abstrakte LLM-Bridge (Spec §5). Implementierungen:
/// - `pf_bridge::PythonBridge` (eingebettetes Python, any-llm/Mock),
/// - `pf_engine::mock::MockBridge` (Rust-seitig, für Unit-Tests).
pub trait LlmBridge: Send + Sync {
    fn complete(&self, request: &LlmRequest) -> Result<LlmResponse>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_display_redacts_api_key() {
        let req = LlmRequest {
            operation: LlmOperation::Chat,
            system_prompt: None,
            user_prompt: "Hallo".to_string(),
            output_schema: None,
            endpoint: Some("http://localhost:11434/v1".to_string()),
            api_key: Some("sk-topsecretvalue".to_string()),
            model: Some("m".to_string()),
            temperature: None,
            max_tokens: None,
            provider: "auto".to_string(),
            request_id: "r1".to_string(),
            timeout_s: None,
        };
        let s = format!("{req}");
        assert!(!s.contains("topsecret"));
    }

    #[test]
    fn strip_fences_extracts_json() {
        let body = "{\"a\":1}";
        assert_eq!(strip_code_fence(&format!("```json\n{body}\n```")), body);
        assert_eq!(strip_code_fence(body), body);
    }

    #[test]
    fn parse_content_json_tolerates_fences() {
        let resp = LlmResponse {
            content: "```json\n{\"ok\": true}\n```".to_string(),
            finish_reason: None,
            usage: None,
            model: None,
            duration_ms: None,
        };
        let v = resp.parse_content_json().unwrap();
        assert_eq!(v["ok"], true);
    }
}
