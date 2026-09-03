//! Verifikation (Spec §7): strukturelle Checks in Rust (deterministisch) +
//! optional LLM-Semantik-Check (Python). Ergebnis strukturiert.
//!
//! Die Datentypen (CheckResult, Verdict, SemanticReport, VerificationReport)
//! leben in pf-core (damit das formatneutrale `CompilationResult` sie ohne
//! pf-engine-Abhängigkeit referenzieren kann) und werden hier re-exportiert,
//! damit bestehende Importe (`pf_engine::verify::…`) unverändert bleiben.

pub use pf_core::verify::{CheckResult, SemanticReport, Verdict, VerificationReport};

use pf_core::Result;
use pf_core::ir::PromptIr;

use crate::optimizer::token_set;

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
    pub context: Vec<String>,
    pub inputs: Vec<String>,
    pub assumptions: Vec<String>,
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
            // v0.2-Optimizer-Regeln: auch Kontext/Eingaben/Annahmen sind
            // verpflichtende Atom-Inhalte (dürfen nicht verloren gehen).
            context: ir
                .context
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            inputs: ir
                .inputs
                .iter()
                .map(|i| {
                    let name = i.name.trim();
                    let desc = i.description.trim();
                    if name.is_empty() {
                        desc.to_string()
                    } else if desc.is_empty() {
                        name.to_string()
                    } else {
                        format!("{name}: {desc}")
                    }
                })
                .filter(|s| !s.is_empty())
                .collect(),
            assumptions: ir
                .assumptions
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
        out.extend(self.context.iter().map(|s| ("context", s)));
        out.extend(self.inputs.iter().map(|s| ("input", s)));
        out.extend(self.assumptions.iter().map(|s| ("assumption", s)));
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
/// Output-Contract, Instructions, Verification Requirements — plus Kontext,
/// Eingaben und Annahmen (v0.2: Optimizer darf keine IR-relevante Information
/// entfernen; Guard stellt verlorene Atome wieder her).
pub fn mandatory_atoms(atoms: &SemanticAtoms) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(atoms.objective.iter().cloned());
    out.extend(atoms.constraints.iter().cloned());
    out.extend(atoms.contract.iter().cloned());
    out.extend(atoms.instructions.iter().cloned());
    out.extend(atoms.requirements.iter().cloned());
    out.extend(atoms.context.iter().cloned());
    out.extend(atoms.inputs.iter().cloned());
    out.extend(atoms.assumptions.iter().cloned());
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

    #[test]
    fn summary_only_is_not_a_pass_and_guard_restores_atoms() {
        // v0.2: Ein Optimierer, der die IR auf eine Zusammenfassung reduziert
        // („Identifizieren von Risiken, …“), darf NICHT bestehen; der Guard
        // stellt verlorene Pflicht-Atome wieder her, erst dann ist PASS ok.
        let mut ir = ir_fixture();
        ir.objective = vec!["Alle Architekturrisiken identifizieren".to_string()];
        ir.context = vec!["Repository: PromptForge (Rust/Python)".to_string()];
        ir.inputs = vec![pf_core::ir::InputSpec {
            name: "Projektpfad".to_string(),
            description: "Lokaler Pfad zum Repository".to_string(),
        }];
        ir.assumptions = vec!["Lokale Entwicklung auf macOS".to_string()];
        let atoms = SemanticAtoms::from_ir(&ir);

        // Reine Zusammenfassung („kurz, aber semantisch schlecht“):
        let summary = "Identifizieren von Risiken, Evaluieren der Projektqualität und Erstellen eines Berichts.\n";
        let bare = verify_structural(&ir, summary, &atoms, 0.85);
        assert_eq!(
            bare.verdict,
            Some(Verdict::Fail),
            "Zusammenfassung darf keinen PASS erhalten"
        );
        assert!(!bare.objective_preserved);

        // Guard stellt alle Pflicht-Atome wieder her:
        let (guarded, ev) = crate::optimizer::reinsert_missing_atoms(
            summary,
            &crate::verify::mandatory_atoms(&atoms),
        );
        assert!(
            matches!(
                ev.action,
                crate::optimizer::PassAction::ReinsertedAtoms(n) if n >= 5
            ),
            "Guard soll mehrere Kategorien wiederherstellen"
        );
        for atom in crate::verify::mandatory_atoms(&atoms) {
            assert!(
                guarded.contains(atom.trim()),
                "Guard hat Atom nicht wiederhergestellt: {atom}"
            );
        }
        let repaired = verify_structural(&ir, &guarded, &atoms, 0.85);
        assert_eq!(
            repaired.verdict,
            Some(Verdict::Pass),
            "Guard-reparierter Prompt darf PASS erhalten"
        );
        // Ein niedrigerer Token-Count allein ist kein Erfolg: semantic bleibt
        // an Atome gebunden.
        assert!(guarded.chars().count() < summary.chars().count() + 4000);
    }
}
