//! Deterministische Expansion: Prompt-IR → ausführlicher Long Prompt.
//!
//! Ziel (Spec §5): Ambiguität reduzieren, Anforderungen explizit machen,
//! Rollen/Ziele/Constraints/Output-Contract/Verifikation sichtbar machen.
//! Token-Minimierung ist hier bewusst NICHT das Ziel.

use pf_core::ir::PromptIr;

/// Erzeugt den Long Prompt aus der IR. Leere Sektionen werden ausgelassen.
pub fn expand_to_long_prompt(ir: &PromptIr) -> String {
    let mut out = String::new();
    out.push_str("# Prompt\n\n");

    if let Some(role) = ir.role.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        push_section(&mut out, "Rolle", [role]);
    }

    if !ir.task.trim().is_empty() {
        push_section(&mut out, "Aufgabe", [ir.task.trim()]);
    }

    if !ir.objective.is_empty() {
        push_section(&mut out, "Ziele", ir.objective.iter().map(String::as_str));
    }

    if !ir.context.is_empty() {
        push_section(&mut out, "Kontext", ir.context.iter().map(String::as_str));
    }

    if !ir.inputs.is_empty() {
        let lines: Vec<String> = ir
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
            .collect();
        push_section(&mut out, "Eingaben", lines.iter().map(String::as_str));
    }

    if !ir.constraints.is_empty() {
        let lines: Vec<String> = ir
            .constraints
            .iter()
            .map(|c| match c.severity {
                pf_core::ConstraintSeverity::Required => format!("(Pflicht) {}", c.text.trim()),
                pf_core::ConstraintSeverity::Recommended => {
                    format!("(Empfohlen) {}", c.text.trim())
                }
            })
            .collect();
        push_section(&mut out, "Constraints", lines.iter().map(String::as_str));
    }

    if !ir.procedure.is_empty() {
        push_section(
            &mut out,
            "Vorgehen",
            ir.procedure.iter().map(String::as_str),
        );
    }

    if let Some(rs) = ir
        .reasoning_strategy
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
    {
        push_section(&mut out, "Denkweise", [rs]);
    }

    if !ir.examples.is_empty() {
        let mut ex = Vec::new();
        for (i, e) in ir.examples.iter().enumerate() {
            ex.push(format!("Beispiel {} — Eingabe: {}", i + 1, e.input.trim()));
            if !e.output.trim().is_empty() {
                ex.push(format!("Beispiel {} — Ausgabe: {}", i + 1, e.output.trim()));
            }
        }
        push_section(&mut out, "Beispiele", ex.iter().map(String::as_str));
    }

    if !ir.assumptions.is_empty() {
        push_section(
            &mut out,
            "Annahmen",
            ir.assumptions.iter().map(String::as_str),
        );
    }

    if !ir.output_contract.format.trim().is_empty() || !ir.output_contract.structure.is_empty() {
        let mut lines = Vec::new();
        if !ir.output_contract.format.trim().is_empty() {
            lines.push(format!("Format: {}", ir.output_contract.format.trim()));
        }
        lines.extend(
            ir.output_contract
                .structure
                .iter()
                .map(String::as_str)
                .map(|s| s.to_string()),
        );
        push_section(&mut out, "Ausgabeformat", lines.iter().map(String::as_str));
    }

    if !ir.output_contract.rules.is_empty() {
        push_section(
            &mut out,
            "Formatregeln",
            ir.output_contract.rules.iter().map(String::as_str),
        );
    }

    if !ir.verification_requirements.is_empty() {
        push_section(
            &mut out,
            "Verifikationsanforderungen",
            ir.verification_requirements.iter().map(String::as_str),
        );
    }

    if let Some(tm) = ir
        .target_model
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        push_section(&mut out, "Zielmodell", [tm]);
    }

    // Sprachliche Leitplanke: in der Sprache der Aufgabe antworten.
    out.push_str("Hinweis: Antworte in derselben Sprache wie die Aufgabe, sofern nichts anderes gefordert ist.\n");

    out
}

fn push_section<'a>(out: &mut String, title: &str, lines: impl IntoIterator<Item = &'a str>) {
    out.push_str("## ");
    out.push_str(title);
    out.push('\n');
    for line in lines {
        let l = line.trim();
        if !l.is_empty() {
            out.push_str("- ");
            out.push_str(l);
            out.push('\n');
        }
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use pf_core::ir::{Constraint, ConstraintSeverity, PromptIr};

    fn sample() -> PromptIr {
        let mut ir = PromptIr::new("rid", "Vergleiche fünf Papers");
        ir.role = Some("Wissenschaftlicher Berater".to_string());
        ir.objective = vec!["Methoden vergleichen".to_string()];
        ir.context = vec!["Wir entwickeln ein Krater-Klassifikationssystem".to_string()];
        ir.constraints = vec![Constraint {
            text: "Nur peer-reviewte Quellen verwenden".to_string(),
            severity: ConstraintSeverity::Required,
        }];
        ir.procedure = vec![
            "Papers zusammenfassen".to_string(),
            "Methoden vergleichen".to_string(),
        ];
        ir.output_contract.format = "markdown".to_string();
        ir.output_contract.structure = vec!["Vergleichstabelle".to_string()];
        ir.output_contract.rules = vec!["Tabelle mit Quellenangaben".to_string()];
        ir.verification_requirements = vec!["Jede Empfehlung begründen".to_string()];
        ir
    }

    #[test]
    fn expansion_contains_all_sections() {
        let text = expand_to_long_prompt(&sample());
        for expected in [
            "# Prompt",
            "## Rolle",
            "## Aufgabe",
            "## Ziele",
            "## Kontext",
            "## Constraints",
            "## Vorgehen",
            "## Ausgabeformat",
            "## Formatregeln",
            "## Verifikationsanforderungen",
        ] {
            assert!(text.contains(expected), "fehlt: {expected}\n---\n{text}");
        }
        assert!(text.contains("(Pflicht) Nur peer-reviewte Quellen verwenden"));
    }

    #[test]
    fn expansion_omits_empty_sections() {
        let ir = PromptIr::new("rid", "Nur Aufgabe");
        let text = expand_to_long_prompt(&ir);
        assert!(text.contains("## Aufgabe"));
        assert!(!text.contains("## Rolle"));
        assert!(!text.contains("## Beispiele"));
        assert!(!text.contains("## Annahmen"));
    }

    #[test]
    fn expansion_is_longer_than_plain_task() {
        let ir = sample();
        let text = expand_to_long_prompt(&ir);
        assert!(text.len() > ir.task.len() + 200);
    }
}
