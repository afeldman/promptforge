//! Prompt IR — versionierte, JSON-serialisierbare Zwischendarstellung
//! (Spec §4). Bewusst provider-unabhängig; alle Stadien der Pipeline
//! arbeiten auf dieser Struktur.

use serde::{Deserialize, Serialize};

use crate::error::{ErrorKind, Result, err};

pub const IR_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintSeverity {
    #[default]
    Required,
    Recommended,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct InputSpec {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Constraint {
    pub text: String,
    #[serde(default)]
    pub severity: ConstraintSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Example {
    pub input: String,
    pub output: String,
}

/// Output-Contract: Was die Antwort erfüllen muss.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct OutputContract {
    /// z. B. "markdown", "text", "json" — freies Feld, von LLM gefüllt.
    pub format: String,
    /// Strukturelemente der Antwort (Abschnitte/Felder).
    pub structure: Vec<String>,
    /// Harte Format-/Inhaltsregeln.
    pub rules: Vec<String>,
    /// Optionales Beispiel („so soll das Ergebnis aussehen“).
    pub example: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct IrMetadata {
    pub request_id: String,
    pub created_at: String,
    pub source_language: Option<String>,
    pub tags: Vec<String>,
    pub engine_version: String,
}

/// Die Prompt-IR (Spec §4: Task, Objective, Context, Inputs, Constraints,
/// Assumptions, Role, Procedure, Reasoning Strategy, Examples, Output
/// Contract, Verification Requirements, Target Model, Metadata).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PromptIr {
    pub schema_version: u32,
    /// Kernaufgabe in einem Satz.
    pub task: String,
    /// Ziele (sortiert nach Wichtigkeit).
    pub objective: Vec<String>,
    /// Relevanter Kontext / Hintergrundinformationen.
    pub context: Vec<String>,
    /// Eingaben, die der Benutzer liefert.
    pub inputs: Vec<InputSpec>,
    /// Constraints: Muss-/Soll-Bedingungen.
    pub constraints: Vec<Constraint>,
    /// Explizite Annahmen.
    pub assumptions: Vec<String>,
    /// Optional: Rolle des LLM.
    pub role: Option<String>,
    /// Geordnete Arbeitsschritte.
    pub procedure: Vec<String>,
    /// Denk-/Argumentationsstrategie (z. B. Schritt-für-Schritt).
    pub reasoning_strategy: Option<String>,
    /// Beispiele (Few-Shot).
    pub examples: Vec<Example>,
    /// Output-Contract.
    pub output_contract: OutputContract,
    /// Verifikationsanforderungen an das Ergebnis.
    pub verification_requirements: Vec<String>,
    /// Zielmodell, falls bekannt.
    pub target_model: Option<String>,
    pub metadata: IrMetadata,
}

impl PromptIr {
    pub fn new(request_id: &str, task: impl Into<String>) -> Self {
        Self {
            schema_version: IR_SCHEMA_VERSION,
            task: task.into(),
            metadata: IrMetadata {
                request_id: request_id.to_string(),
                created_at: crate::path::rfc3339_now(),
                engine_version: env!("CARGO_PKG_VERSION").to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(Into::into)
    }

    pub fn from_json(s: &str) -> Result<Self> {
        let ir: PromptIr = serde_json::from_str(s)?;
        let issues = ir.validate();
        if issues.is_empty() {
            Ok(ir)
        } else {
            Err(err(
                ErrorKind::InvalidInput,
                format!("Ungültige IR: {}", issues.join("; ")),
            ))
        }
    }

    /// Strukturelle Validierung; liefert menschenlesbare Probleme.
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if self.task.trim().is_empty() {
            issues.push("task fehlt".to_string());
        }
        if self.schema_version != IR_SCHEMA_VERSION {
            issues.push(format!(
                "schema_version {} != erwartet {}",
                self.schema_version, IR_SCHEMA_VERSION
            ));
        }
        if self.objective.is_empty() && self.task.trim().is_empty() {
            issues.push("objective fehlt".to_string());
        }
        if self.metadata.request_id.trim().is_empty() {
            issues.push("metadata.request_id fehlt".to_string());
        }
        issues
    }

    /// Deterministische Basis-IR ohne LLM (Spec: Fallback, testbar).
    pub fn from_intent_basic(intent: &str, request_id: &str) -> Self {
        let clean = intent.trim();
        let mut ir = Self::new(request_id, clean);
        ir.objective = vec![format!(
            "Erfülle die folgende Aufgabe vollständig und korrekt: {clean}"
        )];
        ir.output_contract.format = "markdown".to_string();
        ir.output_contract.structure = vec![
            "Ergebnis".to_string(),
            "Begründung (falls sinnvoll)".to_string(),
        ];
        ir.output_contract.rules = vec![
            "Antworte präzise und direkt auf die Aufgabe".to_string(),
            "Keine irrelevante Einleitung".to_string(),
        ];
        ir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ir() -> PromptIr {
        let mut ir = PromptIr::new("rid-1", "Fünf Papers vergleichen");
        ir.objective.push("Methoden vergleichen".to_string());
        ir.constraints.push(Constraint {
            text: "Nur peer-reviewte Quellen".to_string(),
            severity: ConstraintSeverity::Required,
        });
        ir.procedure.push("Zusammenfassen".to_string());
        ir.output_contract.format = "markdown".to_string();
        ir.output_contract.structure = vec!["Vergleichstabelle".to_string()];
        ir
    }

    #[test]
    fn ir_json_roundtrip() {
        let ir = sample_ir();
        let json = ir.to_json().unwrap();
        let back = PromptIr::from_json(&json).unwrap();
        assert_eq!(back, ir);
        assert_eq!(back.schema_version, IR_SCHEMA_VERSION);
        assert_eq!(back.constraints[0].text, "Nur peer-reviewte Quellen");
    }

    #[test]
    fn ir_validate_catches_empty_task() {
        let ir = PromptIr::new("rid-2", "   ");
        let issues = ir.validate();
        assert!(!issues.is_empty());
    }

    #[test]
    fn ir_from_json_rejects_invalid() {
        assert!(PromptIr::from_json(r#"{"task": "x"}"#).is_err());
    }

    #[test]
    fn basic_ir_has_objective_and_output_contract() {
        let ir = PromptIr::from_intent_basic("  Analysiere die Papers  ", "rid-3");
        assert_eq!(ir.task, "Analysiere die Papers");
        assert_eq!(ir.objective.len(), 1);
        assert_eq!(ir.output_contract.format, "markdown");
        assert!(ir.validate().is_empty());
    }
}
