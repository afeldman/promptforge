//! Verifikations-Datentypen (Spec §7) — in pf-core, damit das
//! formatneutrale `CompilationResult` (v0.2) sie referenzieren kann, ohne
//! von pf-engine abzuhängen. Die Verifikations-Logik (strukturelle Checks,
//! Semantik-Atome, Merge) lebt weiterhin in pf-engine::verify.
//!
//! Strukturiertes Ergebnis (Beispiel Spec §7):
//!   semantic_preservation: 0.98, constraints_preserved: true, …,
//!   verdict: PASS

use serde::{Deserialize, Serialize};

/// Ein einzelner Semantik-Atom-Check (Atom = zu erhaltender Textbaustein).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CheckResult {
    pub category: String,
    pub atom: String,
    pub ok: bool,
    /// Anteil der Atom-Tokens, die im Zieltext enthalten sind (0..1).
    pub ratio: f64,
}

/// Verdict der Verifikation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    #[default]
    Fail,
}

impl Verdict {
    pub fn is_pass(self) -> bool {
        self == Verdict::Pass
    }
}

/// LLM-Semantik-Bericht (Python `verify`-Operation).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SemanticReport {
    pub semantic_preservation: f64,
    pub constraints_preserved: bool,
    pub output_contract_preserved: bool,
    pub objective_preserved: bool,
    pub instructions_preserved: bool,
    #[serde(default)]
    pub comment: String,
}

/// Vollständiger Verifikationsbericht.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct VerificationReport {
    /// Gesamt-Semantik-Erhalt (0..1).
    pub semantic_preservation: f64,
    pub constraints_preserved: bool,
    pub output_contract_preserved: bool,
    pub objective_preserved: bool,
    pub instructions_preserved: bool,
    pub verification_requirements_preserved: bool,
    pub verdict: Option<Verdict>,
    #[serde(default)]
    pub checks: Vec<CheckResult>,
    /// Anzahl Verifikations-/Retry-Läufe.
    pub attempts: u32,
    #[serde(default)]
    pub details: Vec<String>,
}

impl VerificationReport {
    /// Sind alle Erhaltungs-Booleans gesetzt (strukturelle Gültigkeit)?
    pub fn all_preserved(&self) -> bool {
        self.constraints_preserved
            && self.output_contract_preserved
            && self.objective_preserved
            && self.instructions_preserved
            && self.verification_requirements_preserved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_report_roundtrip() {
        let report = VerificationReport {
            semantic_preservation: 0.98,
            constraints_preserved: true,
            output_contract_preserved: true,
            objective_preserved: true,
            instructions_preserved: true,
            verification_requirements_preserved: true,
            verdict: Some(Verdict::Pass),
            checks: vec![CheckResult {
                category: "constraint".to_string(),
                atom: "Nur peer-reviewte Quellen".to_string(),
                ok: true,
                ratio: 1.0,
            }],
            attempts: 2,
            details: vec!["LLM-Semantik: ok".to_string()],
        };
        let json = serde_json::to_string(&report).unwrap();
        let back: VerificationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
        assert_eq!(back.verdict, Some(Verdict::Pass));
    }

    #[test]
    fn verdict_snake_case_and_default() {
        let v: Verdict = serde_json::from_str(r#""pass""#).unwrap();
        assert_eq!(v, Verdict::Pass);
        assert_eq!(serde_json::to_string(&Verdict::Pass).unwrap(), r#""pass""#);
        assert_eq!(Verdict::default(), Verdict::Fail);
        assert!(Verdict::Pass.is_pass());
        assert!(!Verdict::Fail.is_pass());
    }

    #[test]
    fn report_defaults_and_all_preserved() {
        let r = VerificationReport::default();
        assert!(r.checks.is_empty());
        assert_eq!(r.verdict, None);
        assert!(!r.all_preserved()); // alles false → nicht erhalten
        let ok = VerificationReport {
            constraints_preserved: true,
            output_contract_preserved: true,
            objective_preserved: true,
            instructions_preserved: true,
            verification_requirements_preserved: true,
            ..Default::default()
        };
        assert!(ok.all_preserved());
    }
}
