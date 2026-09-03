//! v0.2: Formatneutrales Kompilier-Ergebnis `CompilationResult` (Design §4)
//! plus Qualitäts-Metriken `QualityMetrics` (Design §18, Minimalumfang für
//! Phase 1).
//!
//! Wichtig: `CompilationResult` kennt KEINE Ausgabeformate (text/json/yaml/
//! toon). Serializer kommen in Phase 2 und lesen nur dieses Objekt.

use serde::{Deserialize, Serialize};

use crate::ir::PromptIr;
use crate::token::TokenReport;
use crate::verify::VerificationReport;

/// Qualitäts-Metriken einer Kompilierung (v0.2, Phase-1-Minimalumfang).
/// Bewusst mehrere Achsen statt eines einzigen Scores (Design §18):
///
/// ```text
/// semantic_fidelity = 1.00
/// token_efficiency  = -0.42   (Prompt wurde größer → kein stiller Erfolg)
/// ```
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub struct QualityMetrics {
    /// Semantik-Erhalt (0..1); = semantic_preservation des Verifikationsberichts.
    pub semantic_fidelity: f64,
    /// Strukturelle Gültigkeit: alle Erhaltungs-Kategorien ok.
    pub structural_validity: bool,
    /// Token-Effizienz relativ: 1 - optimized/generated.
    /// > 0 kleiner, = 0 gleich, < 0 größer (0 wenn nichts generiert wurde).
    pub token_efficiency: f64,
}

impl QualityMetrics {
    /// Deterministische Berechnung aus Verifikationsbericht + Token-Report.
    pub fn compute(verification: &VerificationReport, token_report: &TokenReport) -> Self {
        let token_efficiency = if token_report.generated > 0 {
            1.0 - (token_report.optimized as f64 / token_report.generated as f64)
        } else {
            0.0
        };
        Self {
            semantic_fidelity: verification.semantic_preservation,
            structural_validity: verification.all_preserved(),
            token_efficiency,
        }
    }
}

/// Vollständiges, formatneutrales Kompilier-Ergebnis der Engine (v0.2).
///
/// Ersetzt das interne v0.1-`CompileOutcome` als Rückgabetyp der Engine und
/// ist die einzige Ergebnisstruktur für CLI/TUI/Service und spätere
/// Serializer. Keine zweite Pipeline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompilationResult {
    /// Original-Eingabe (Intent, getrimmt).
    pub input: String,
    pub request_id: String,
    /// Wurde ein LLM (Prompt Generator) verwendet?
    pub llm_used: bool,
    /// Abgeschlossene Pipeline-Stadien.
    pub stages: Vec<String>,
    /// Prompt-IR (semantische Wahrheit).
    pub prompt_ir: PromptIr,
    /// Langprompt aus der Expansion.
    pub expanded_prompt: String,
    /// Finaler optimierter Prompt.
    pub optimized_prompt: String,
    pub token_report: TokenReport,
    pub verification: VerificationReport,
    /// v0.2-Qualitätsmetriken (Phase-1-Minimalumfang).
    pub metrics: QualityMetrics,
}

impl CompilationResult {
    /// JSON-Envelope: kanonische v0.2-Felder (Serde-Namen) plus v0.1-Aliase
    /// (`ir`, `long_prompt`, `final_output`) für Abwärtskompatibilität mit
    /// bestehenden Konsumenten (CLI `--json`, Service, smoke.sh).
    pub fn envelope_json(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(self).unwrap_or_else(|_| serde_json::json!({}));
        // v0.1-Aliase: gleiche Werte unter alten Schlüsseln.
        if let Some(obj) = v.as_object_mut() {
            obj.insert(
                "ir".to_string(),
                serde_json::to_value(&self.prompt_ir).unwrap_or(serde_json::Value::Null),
            );
            obj.insert(
                "long_prompt".to_string(),
                serde_json::Value::String(self.expanded_prompt.clone()),
            );
            obj.insert(
                "final_output".to_string(),
                serde_json::Value::String(self.optimized_prompt.clone()),
            );
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Constraint, ConstraintSeverity};
    use crate::token::TokenReport;
    use crate::verify::{Verdict, VerificationReport};

    fn sample_ir(request_id: &str) -> PromptIr {
        let mut ir = PromptIr::new(request_id, "Auditiere das Projekt");
        ir.objective = vec!["Architektur prüfen".to_string()];
        ir.constraints = vec![Constraint {
            text: "Keine erfundenen Befunde".to_string(),
            severity: ConstraintSeverity::Required,
        }];
        ir
    }

    fn sample_verification() -> VerificationReport {
        VerificationReport {
            semantic_preservation: 0.98,
            constraints_preserved: true,
            output_contract_preserved: true,
            objective_preserved: true,
            instructions_preserved: true,
            verification_requirements_preserved: true,
            verdict: Some(Verdict::Pass),
            attempts: 1,
            ..Default::default()
        }
    }

    fn sample_token_report() -> TokenReport {
        let mut t = TokenReport::new();
        t.set_original(10);
        t.set_generated(100);
        t.set_optimized(60);
        t
    }

    fn sample_result() -> CompilationResult {
        CompilationResult {
            input: "Auditiere das Projekt".to_string(),
            request_id: "rid-c1".to_string(),
            llm_used: false,
            stages: vec![
                "architect".to_string(),
                "expand".to_string(),
                "optimize".to_string(),
                "verify".to_string(),
            ],
            prompt_ir: sample_ir("rid-c1"),
            expanded_prompt: "## Aufgabe\n…".to_string(),
            optimized_prompt: "## Aufgabe\n… optimiert".to_string(),
            token_report: sample_token_report(),
            verification: sample_verification(),
            metrics: QualityMetrics::default(),
        }
    }

    #[test]
    fn construction_roundtrip() {
        let result = sample_result();
        let json = serde_json::to_string(&result).unwrap();
        let back: CompilationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, result);
        assert_eq!(back.input, "Auditiere das Projekt");
        assert_eq!(back.prompt_ir.task, "Auditiere das Projekt");
        assert_eq!(back.stages.len(), 4);
        assert_eq!(back.verification.verdict, Some(Verdict::Pass));
    }

    #[test]
    fn envelope_json_keeps_legacy_keys() {
        let result = sample_result();
        let v = result.envelope_json();
        // v0.1-Aliase
        assert!(v.get("ir").is_some());
        assert_eq!(v["long_prompt"], "## Aufgabe\n…");
        assert_eq!(v["final_output"], "## Aufgabe\n… optimiert");
        assert_eq!(v["optimized_prompt"], "## Aufgabe\n… optimiert");
        assert!(v.get("token_report").is_some());
        assert!(v.get("verification").is_some());
        // v0.2-kanonisch
        assert_eq!(v["input"], "Auditiere das Projekt");
        assert!(v.get("prompt_ir").is_some());
        assert!(v.get("expanded_prompt").is_some());
        assert!(v.get("metrics").is_some());
    }

    #[test]
    fn metrics_compute_positive_efficiency() {
        let m = QualityMetrics::compute(&sample_verification(), &sample_token_report());
        assert!((m.semantic_fidelity - 0.98).abs() < 1e-9);
        assert!(m.structural_validity);
        assert!((m.token_efficiency - 0.4).abs() < 1e-9); // 1 - 60/100
    }

    #[test]
    fn metrics_compute_negative_efficiency_not_hidden() {
        // Prompt wurde größer (Modell appendet): -0.42 wie im Design-Beispiel.
        let mut token_report = sample_token_report();
        token_report.set_generated(100);
        token_report.set_optimized(142);
        let m = QualityMetrics::compute(&sample_verification(), &token_report);
        assert!((m.token_efficiency - (-0.42)).abs() < 1e-9);
        assert!(m.token_efficiency < 0.0);
    }

    #[test]
    fn metrics_compute_zero_when_nothing_generated() {
        let token_report = TokenReport::new(); // generated = 0
        let m = QualityMetrics::compute(&sample_verification(), &token_report);
        assert_eq!(m.token_efficiency, 0.0);
    }

    #[test]
    fn metrics_structural_invalid_when_boolean_false() {
        let v = VerificationReport {
            constraints_preserved: false,
            ..sample_verification()
        };
        let m = QualityMetrics::compute(&v, &sample_token_report());
        assert!(!m.structural_validity);
    }

    #[test]
    fn result_default_fields_are_optional_serde() {
        // Deserialisierung ohne optionale/fehlende Felder schlägt nur bei
        // Pflichtfeldern fehl; `analysis` in der IR darf fehlen.
        let mut result = sample_result();
        result.prompt_ir.analysis = None;
        let json = serde_json::to_string(&result).unwrap();
        let back: CompilationResult = serde_json::from_str(&json).unwrap();
        assert!(back.prompt_ir.analysis.is_none());
    }
}
