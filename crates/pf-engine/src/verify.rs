//! Verifikation (Spec §7): strukturelle Checks in Rust (deterministisch) +
//! optional LLM-Semantik-Check (Python). Ergebnis strukturiert.

use serde::{Deserialize, Serialize};

use pf_core::Result;
use pf_core::ir::PromptIr;

use crate::optimizer::token_set;

/// Ein einzelner Semantik-Atom-Check (Atom = zu erhaltender Textbaustein).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CheckResult {
    pub category: String,
    pub atom: String,
    pub ok: bool,
    /// Anteil der Atom-Tokens, die im Zieltext enthalten sind (0..1).
    pub ratio: f64,
}

/// Verdict der strukturellen Verifikation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
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

/// Mindest-Ratio pro Atom, ab der es als erhalten gilt.
const ATOM_OK_RATIO: f64 = 0.75;

/// Extrahierte Semantik-Atome aus der IR.
#[derive(Debug, Clone)]
pub struct SemanticAtoms {
    pub objective: Vec<String>,
    pub constraints: Vec<String>,
    pub contract: Vec<String>,
    pub instructions: Vec<String>,
    pub requirements: Vec<String>,
}

impl SemanticAtoms {
    pub fn from_ir(ir: &PromptIr) -> Self {
        Self {
            objective: ir
                .objective
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            constraints: ir
                .constraints
                .iter()
                .map(|c| c.text.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            contract: ir
                .output_contract
                .structure
                .iter()
                .chain(ir.output_contract.rules.iter())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            instructions: ir
                .procedure
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            requirements: ir
                .verification_requirements
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }

    pub fn all(&self) -> Vec<(&'static str, &String)> {
        let mut out = Vec::new();
        out.extend(self.objective.iter().map(|s| ("objective", s)));
        out.extend(self.constraints.iter().map(|s| ("constraint", s)));
        out.extend(self.contract.iter().map(|s| ("output_contract", s)));
        out.extend(self.instructions.iter().map(|s| ("instruction", s)));
        out.extend(
            self.requirements
                .iter()
                .map(|s| ("verification_requirement", s)),
        );
        out
    }
}

fn atom_ratio(atom: &str, target_tokens: &std::collections::BTreeSet<String>) -> f64 {
    let atom_tokens = token_set(atom);
    if atom_tokens.is_empty() {
        return 1.0;
    }
    let matched = atom_tokens
        .iter()
        .filter(|t| target_tokens.contains(*t))
        .count();
    matched as f64 / atom_tokens.len() as f64
}

/// Leere Kategorien gelten als erhalten (vakant wahr).
fn category_ok(checks: &[&CheckResult]) -> bool {
    checks.iter().all(|c| c.ok)
}

/// Deterministische Struktur-Verifikation eines optimierten Prompts gegen
/// die IR (Spec §7). Liefert Report mit gesetztem Verdict.
pub fn verify_structural(
    ir: &PromptIr,
    optimized: &str,
    atoms: &SemanticAtoms,
    threshold: f64,
) -> VerificationReport {
    let target = token_set(optimized);
    let mut checks: Vec<CheckResult> = Vec::new();
    for (category, atom) in atoms.all() {
        let ratio = atom_ratio(atom, &target);
        checks.push(CheckResult {
            category: category.to_string(),
            atom: atom.clone(),
            ok: ratio >= ATOM_OK_RATIO,
            ratio,
        });
    }
    let cat = |c: &str| {
        checks
            .iter()
            .filter(|x| x.category == c)
            .collect::<Vec<_>>()
    };
    let objective = cat("objective");
    let constraints = cat("constraint");
    let contract = cat("output_contract");
    let instructions = cat("instruction");
    let requirements = cat("verification_requirement");

    let semantic_preservation = if checks.is_empty() {
        1.0
    } else {
        checks.iter().map(|c| c.ratio).sum::<f64>() / checks.len() as f64
    };

    let report = VerificationReport {
        semantic_preservation,
        constraints_preserved: category_ok(&constraints),
        output_contract_preserved: category_ok(&contract),
        objective_preserved: category_ok(&objective),
        instructions_preserved: category_ok(&instructions),
        verification_requirements_preserved: category_ok(&requirements),
        verdict: None,
        checks,
        attempts: 1,
        details: vec!["strukturelle Verifikation (Rust)".to_string()],
    };

    let verdict = if report.semantic_preservation >= threshold
        && report.constraints_preserved
        && report.output_contract_preserved
        && report.objective_preserved
        && report.instructions_preserved
        && report.verification_requirements_preserved
    {
        Verdict::Pass
    } else {
        Verdict::Fail
    };
    let _ = ir;
    VerificationReport {
        verdict: Some(verdict),
        ..report
    }
}

/// Merged einen LLM-Semantik-Bericht in die Struktur-Verifikation.
pub fn merge_semantic(
    mut structural: VerificationReport,
    llm: SemanticReport,
    threshold: f64,
) -> VerificationReport {
    structural.semantic_preservation = llm.semantic_preservation;
    structural.constraints_preserved &= llm.constraints_preserved;
    structural.output_contract_preserved &= llm.output_contract_preserved;
    structural.objective_preserved &= llm.objective_preserved;
    structural.instructions_preserved &= llm.instructions_preserved;
    structural.attempts += 1;
    if !llm.comment.is_empty() {
        structural
            .details
            .push(format!("LLM-Semantik: {}", llm.comment));
    }
    structural.verdict = Some(
        if structural.semantic_preservation >= threshold
            && structural.constraints_preserved
            && structural.output_contract_preserved
            && structural.objective_preserved
            && structural.instructions_preserved
            && structural.verification_requirements_preserved
        {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
    );
    structural
}

/// Konvertiert die IR in ein kompaktes JSON für LLM-Verify-Payloads.
pub fn atoms_payload(atoms: &SemanticAtoms) -> serde_json::Value {
    serde_json::json!({
        "objective": atoms.objective,
        "constraints": atoms.constraints,
        "output_contract": atoms.contract,
        "instructions": atoms.instructions,
        "verification_requirements": atoms.requirements,
    })
}

/// Pflicht-Atome für den Reinsert-Guard-Pass: Objective, Constraints,
/// Output-Contract und Instructions werden strukturell erhalten — der
/// Guard verhindert, dass wichtige Informationen durch die Optimierung
/// verloren gehen (Spec §6/§7).
pub fn mandatory_atoms(atoms: &SemanticAtoms) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(atoms.objective.iter().cloned());
    out.extend(atoms.constraints.iter().cloned());
    out.extend(atoms.contract.iter().cloned());
    out.extend(atoms.instructions.iter().cloned());
    out.extend(atoms.requirements.iter().cloned());
    out
}

/// Parse eines VerificationReport-JSON aus der LLM-Schicht (Python `verify`).
pub fn parse_semantic_report_json(value: &serde_json::Value) -> Result<SemanticReport> {
    let report: SemanticReport = serde_json::from_value(value.clone())?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pf_core::ir::{Constraint, ConstraintSeverity};

    fn ir_fixture() -> PromptIr {
        let mut ir = PromptIr::new("rid", "Vergleiche fünf Papers");
        ir.objective = vec!["Methoden vergleichen".to_string()];
        ir.constraints = vec![Constraint {
            text: "Nur peer-reviewte Quellen verwenden".to_string(),
            severity: ConstraintSeverity::Required,
        }];
        ir.procedure = vec!["Papers zusammenfassen".to_string()];
        ir.output_contract.structure = vec!["Vergleichstabelle".to_string()];
        ir.output_contract.rules = vec!["Tabelle mit Quellenangaben".to_string()];
        ir.verification_requirements = vec!["Jede Empfehlung begründen".to_string()];
        ir
    }

    #[test]
    fn structural_pass_on_faithful_text() {
        let ir = ir_fixture();
        let atoms = SemanticAtoms::from_ir(&ir);
        // Text enthält alle Atome wörtlich:
        let text = "## Aufgabe\nVergleiche fünf Papers\n## Ziele\nMethoden vergleichen\n## Constraints\nNur peer-reviewte Quellen verwenden\n## Vorgehen\nPapers zusammenfassen\n## Ausgabeformat\nVergleichstabelle\n## Formatregeln\nTabelle mit Quellenangaben\n## Verifikation\nJede Empfehlung begründen\n";
        let rep = verify_structural(&ir, text, &atoms, 0.85);
        assert_eq!(rep.verdict, Some(Verdict::Pass));
        assert!(rep.constraints_preserved);
        assert!(rep.output_contract_preserved);
    }

    #[test]
    fn structural_fail_when_constraint_missing() {
        let ir = ir_fixture();
        let atoms = SemanticAtoms::from_ir(&ir);
        let text = "## Aufgabe\nVergleiche fünf Papers\n## Ziele\nMethoden vergleichen\n";
        let rep = verify_structural(&ir, text, &atoms, 0.85);
        assert_eq!(rep.verdict, Some(Verdict::Fail));
        assert!(!rep.constraints_preserved);
        let constraint_checks: Vec<_> = rep
            .checks
            .iter()
            .filter(|c| c.category == "constraint")
            .collect();
        assert!(!constraint_checks.is_empty());
        assert!(!constraint_checks[0].ok);
    }

    #[test]
    fn merge_semantic_ands_booleans() {
        let ir = ir_fixture();
        let atoms = SemanticAtoms::from_ir(&ir);
        let text = "## Aufgabe\nVergleiche fünf Papers\n## Ziele\nMethoden vergleichen\n## Constraints\nNur peer-reviewte Quellen verwenden\n## Vorgehen\nPapers zusammenfassen\n## Ausgabeformat\nVergleichstabelle\n## Formatregeln\nTabelle mit Quellenangaben\n## Verifikation\nJede Empfehlung begründen\n";
        let structural = verify_structural(&ir, text, &atoms, 0.85);
        let llm = SemanticReport {
            semantic_preservation: 0.98,
            constraints_preserved: false, // LLM meldet Verlust
            ..Default::default()
        };
        let merged = merge_semantic(structural, llm, 0.85);
        assert_eq!(merged.verdict, Some(Verdict::Fail));
        assert!(!merged.constraints_preserved);
        assert_eq!(merged.attempts, 2);
    }

    #[test]
    fn atoms_payload_is_json() {
        let ir = ir_fixture();
        let atoms = SemanticAtoms::from_ir(&ir);
        let v = atoms_payload(&atoms);
        assert!(v["constraints"].as_array().unwrap().len() == 1);
    }
}
