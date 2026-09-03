//! MockBridge — deterministischer Fake-LLM für Engine-Tests (Rust-seitig,
//! ohne Python). Hält denselben JSON-Vertrag wie die Python-Schicht ein.

use pf_core::bridge::{LlmBridge, LlmOperation, LlmRequest, LlmResponse, Usage};
use pf_core::error::{ErrorKind, Result, err};
use pf_core::ir::PromptIr;

/// Fake-LLM: liefert pro Operation strukturierte, deterministische Antworten.
#[derive(Debug, Clone)]
pub struct MockBridge;

impl MockBridge {
    pub fn new() -> Self {
        Self
    }

    fn architect(&self, user_prompt: &str) -> String {
        // user_prompt ist JSON {"intent": "..."}; bei Parse-Fehler Fallback.
        let intent = serde_json::from_str::<serde_json::Value>(user_prompt)
            .ok()
            .and_then(|v| v.get("intent").and_then(|s| s.as_str()).map(str::to_string))
            .unwrap_or_else(|| user_prompt.to_string());
        let mut ir = PromptIr::from_intent_basic(&intent, "mock");
        ir.role = Some("Wissenschaftlicher Berater (Mock)".to_string());
        ir.objective = vec![format!("Erfülle die Aufgabe vollständig: {intent}")];
        ir.reasoning_strategy = Some("Schritt-für-Schritt-Analyse".to_string());
        serde_json::to_string_pretty(&ir).unwrap_or_else(|_| "{}".to_string())
    }

    fn optimize(&self, user_prompt: &str) -> String {
        // user_prompt = JSON {"ir":…, "long_prompt": "…", "feedback": …}
        let v: serde_json::Value =
            serde_json::from_str(user_prompt).unwrap_or(serde_json::Value::Null);
        let long = v
            .get("long_prompt")
            .and_then(|s| s.as_str())
            .unwrap_or(user_prompt);
        // Mock-Optimierung: nur Whitespace-Normalisierung (Inhalt bleibt).
        let mut collapsed = false;
        let text: String = long
            .lines()
            .filter(|l| {
                let t = l.trim();
                if t.is_empty() {
                    if collapsed {
                        return false;
                    }
                    collapsed = true;
                    true
                } else {
                    collapsed = false;
                    true
                }
            })
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!("{text}\n");
        serde_json::json!({ "prompt": prompt, "notes": ["mock: whitespace"] }).to_string()
    }

    fn verify(&self) -> String {
        serde_json::json!({
            "semantic_preservation": 0.98,
            "constraints_preserved": true,
            "output_contract_preserved": true,
            "objective_preserved": true,
            "instructions_preserved": true,
            "comment": "mock: semantisch erhalten"
        })
        .to_string()
    }

    fn chat(&self, user_prompt: &str) -> String {
        format!(
            "Mock-Antwort auf: {}",
            user_prompt.chars().take(80).collect::<String>()
        )
    }
}

impl LlmBridge for MockBridge {
    fn complete(&self, request: &LlmRequest) -> Result<LlmResponse> {
        let content = match request.operation {
            LlmOperation::Architect => self.architect(&request.user_prompt),
            LlmOperation::Optimize => self.optimize(&request.user_prompt),
            LlmOperation::Verify => self.verify(),
            LlmOperation::Chat => self.chat(&request.user_prompt),
        };
        if request.operation == LlmOperation::Architect
            && serde_json::from_str::<serde_json::Value>(&content).is_err()
        {
            return Err(err(ErrorKind::Model, "Mock-Architect lieferte kein JSON"));
        }
        let completion_tokens = content.chars().count() as u64;
        Ok(LlmResponse {
            content,
            finish_reason: Some("stop".to_string()),
            usage: Some(Usage {
                prompt_tokens: 10,
                completion_tokens,
                total_tokens: 20,
            }),
            model: request.model.clone().or_else(|| Some("mock".to_string())),
            duration_ms: Some(1),
        })
    }
}

impl Default for MockBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pf_core::bridge::LlmOperation;

    fn req(op: LlmOperation, payload: &str) -> LlmRequest {
        LlmRequest {
            operation: op,
            system_prompt: None,
            user_prompt: payload.to_string(),
            output_schema: None,
            endpoint: None,
            api_key: None,
            model: Some("mock".to_string()),
            temperature: None,
            max_tokens: None,
            provider: "mock".to_string(),
            request_id: "r".to_string(),
            timeout_s: None,
        }
    }

    #[test]
    fn mock_architect_returns_valid_ir_json() {
        let b = MockBridge::new();
        let payload = serde_json::json!({ "intent": "Analysiere fünf Papers" }).to_string();
        let resp = b.complete(&req(LlmOperation::Architect, &payload)).unwrap();
        let ir = PromptIr::from_json(&resp.content).unwrap();
        assert!(ir.task.contains("Analysiere fünf Papers"));
        assert_eq!(ir.schema_version, pf_core::IR_SCHEMA_VERSION);
    }

    #[test]
    fn mock_optimize_returns_prompt_field() {
        let b = MockBridge::new();
        let payload = serde_json::json!({ "long_prompt": "Zeile\n\n\nZeile\n" }).to_string();
        let resp = b.complete(&req(LlmOperation::Optimize, &payload)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&resp.content).unwrap();
        assert!(v["prompt"].as_str().unwrap().contains("Zeile"));
    }

    #[test]
    fn mock_verify_returns_semantic_fields() {
        let b = MockBridge::new();
        let resp = b.complete(&req(LlmOperation::Verify, "{}")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&resp.content).unwrap();
        assert_eq!(v["semantic_preservation"], 0.98);
        assert_eq!(v["constraints_preserved"], true);
    }
}
