//! v1.0 Optimization-Strategien (deterministisch) + Technical-Token-Schutz.
//!
//! Konzept (angelehnt an Caveman-Prinzipien, eigene Implementierung):
//! „Code, Commands, Pfade und exakte Fehlertexte werden nie komprimiert —
//! nur die Prosa darum." PromptForge übernimmt daraus die Trennung von
//! schützenswerten technischen Segmenten und sprachlicher Redundanz; alle
//! Kandidaten werden gegen die Prompt IR (kanonische Quelle) verifiziert.
//!
//! Strategien:
//!   redundancy  — Füll- und Subsumptions-Redundanz entfernen
//!   instruction — Prosa-Hedges zu Imperativen verdichten
//!   structural  — kompakte ausführbare Struktur direkt aus der IR
//!   semantic    — konservative semantische Subsumption (Beweis via Tokens)
//!   combined    — redundancy + instruction + semantic
//!
//! Jede Strategie ist deterministisch; LLM-basierte Kompression orchestriert
//! die Pipeline separat (LlmOperation::Optimize) als eigenen Kandidaten.

use pf_core::ir::PromptIr;

use crate::optimizer::token_set;

/// Technische/geschützte Zeile (Code, Fences, Pfade, URLs, Versionen,
/// Command-ähnliche Inhalte). Diese Segmente dürfen nicht sprachlich
/// verändert werden (Caveman-Prinzip, eigene Umsetzung).
pub fn is_protected_line(line: &str) -> bool {
    let t = line.trim_start();
    if t.is_empty() {
        return false;
    }
    // Code-Fences / Inline-Code / eingerückter Code (rohe Zeile prüfen —
    // trim_start würde die Einrückung wegwerfen)
    if line.starts_with("    ") || line.starts_with('\t') || t.starts_with("```") {
        return true;
    }
    if t.contains('`') {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    // URLs
    if lower.contains("://") || lower.starts_with("http") || lower.starts_with("www.") {
        return true;
    }
    // Pfade / Dateien mit typischen Endungen
    if t.contains('/') || t.contains('\\') {
        return true;
    }
    for ext in [
        ".rs", ".py", ".toml", ".json", ".yaml", ".yml", ".sh", ".md", ".c", ".h", ".go", ".js",
        ".ts", ".lock",
    ] {
        if lower.contains(ext) {
            return true;
        }
    }
    // Versionen & Identifiers
    let has_version = t
        .split_whitespace()
        .any(|w| w.starts_with('v') || w.contains('.'))
        && t.split_whitespace().any(|w| {
            w.chars().any(|c| c.is_ascii_digit())
                && (w.contains('.') || w.starts_with('v') || w.contains('-'))
        });
    if has_version {
        return true;
    }
    // CLI-/Command-ähnliche Zeilen (Flag-Muster)
    if t.starts_with('-') && (t.contains('=') || t.len() > 6) && !t.starts_with("- ") {
        return true;
    }
    false
}

/// Entfernt Prosa-Redundanz auf Zeilenebene:
/// 1. reine Diskurs-Zeilen ohne relevante Token,
/// 2. Zeilen, deren Token-Menge Teilmenge einer längeren/anderen Zeile ist
///    (Subsumption — semantisch bewiesen durch Token-Containment).
///
/// Geschützte Zeilen werden nie entfernt (außer sie sind exakt identisch mit
/// einer anderen geschützten Zeile).
pub fn strategy_redundancy(text: &str) -> (String, u64) {
    let lines: Vec<&str> = text.lines().collect();
    let mut removed = 0u64;
    let mut kept: Vec<&str> = Vec::new();
    let mut kept_tokens: Vec<(String, std::collections::BTreeSet<String>)> = Vec::new();
    let mut kept_protected: Vec<String> = Vec::new();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let toks = token_set(trimmed);
        // Reine Diskurs-Zeilen (< 2 Inhaltswörter, kein Schutz) entfernen.
        if toks.len() < 2 && !is_protected_line(trimmed) {
            removed += 1;
            continue;
        }
        let protected = is_protected_line(trimmed);
        // Exakte Duplikate geschützter Zeilen entfernen (ein Vorkommen bleibt).
        if protected {
            if kept_protected.iter().any(|p| p == trimmed) {
                removed += 1;
                continue;
            }
            kept_protected.push(trimmed.to_string());
        }
        // Subsumptions-Check: eine andere längere Zeile enthält alle Tokens.
        if !protected && toks.len() >= 2 {
            let subsumed = kept_tokens.iter().any(|(other, other_toks)| {
                other.chars().count() >= trimmed.chars().count()
                    && !other_toks.is_empty()
                    && toks.is_subset(other_toks)
            });
            if subsumed {
                removed += 1;
                continue;
            }
        }
        kept.push(trimmed);
        kept_tokens.push((trimmed.to_string(), toks));
    }
    (join_lines(kept), removed)
}

/// Verdichtet Prosa-Hedges zu Imperativen (nur ungeschützte Zeilen).
/// Konservative Ersetzungsliste — keine technischen Segmente anfassen.
pub fn strategy_instruction(text: &str) -> (String, u64) {
    const HEDGES: &[(&str, &str)] = &[
        ("please carefully ", ""),
        ("please ", ""),
        ("kindly ", ""),
        ("make sure that you ", ""),
        ("make sure to ", ""),
        ("you need to ", ""),
        ("you should ", ""),
        ("you must ", ""),
        ("be sure to ", ""),
        ("it is important to ", ""),
        ("it is important that ", ""),
        ("in order to ", "to "),
        ("that could potentially ", "that could "),
        ("could potentially cause ", "could cause "),
        ("potentially cause ", "cause "),
        (" as well", ""),
    ];
    let mut removed = 0u64;
    let mut out_lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_end();
        if is_protected_line(line) {
            out_lines.push(line.to_string());
            continue;
        }
        let trimmed = line.trim();
        let mut changed = trimmed.to_string();
        let lower = changed.to_ascii_lowercase();
        for (from, to) in HEDGES {
            let from_l = from.to_ascii_lowercase();
            let mut candidate = changed.clone();
            // Einfacher, wiederholter Ersatz (ohne Regex-Abhängigkeit).
            loop {
                let cl = candidate.to_ascii_lowercase();
                if let Some(idx) = cl.find(&from_l) {
                    candidate.replace_range(idx..idx + from_l.len(), to);
                } else {
                    break;
                }
            }
            let _ = lower;
            if candidate != changed {
                // Nicht unter 3 Inhaltswörter kürzen (Schutz vor Zerstörung).
                if token_set(&candidate).len() >= 3 || candidate.trim().is_empty() {
                    changed = candidate;
                }
            }
        }
        if changed != trimmed {
            removed += 1;
        }
        out_lines.push(changed.to_string());
    }
    (join_string_lines(out_lines), removed)
}

/// Structural Compression: kompakte ausführbare Struktur direkt aus der
/// Prompt IR (kanonische Quelle). Alle Pflicht-Inhalte werden wörtlich
/// übernommen — nur Boilerplate entfällt.
pub fn strategy_structural(ir: &PromptIr) -> String {
    let mut out: Vec<String> = Vec::new();
    push(&mut out, "AUFGABE", &[ir.task.trim().to_string()]);
    if !ir.objective.is_empty() {
        push(
            &mut out,
            "ZIELE",
            &ir.objective
                .iter()
                .map(|s| format!("- {s}"))
                .collect::<Vec<_>>(),
        );
    }
    if !ir.context.is_empty() {
        push(
            &mut out,
            "KONTEXT",
            &ir.context
                .iter()
                .map(|s| format!("- {s}"))
                .collect::<Vec<_>>(),
        );
    }
    if !ir.inputs.is_empty() {
        let lines: Vec<String> = ir
            .inputs
            .iter()
            .map(|i| {
                if i.description.trim().is_empty() {
                    format!("- {}", i.name.trim())
                } else {
                    format!("- {}: {}", i.name.trim(), i.description.trim())
                }
            })
            .collect();
        push(&mut out, "EINGABEN", &lines);
    }
    if !ir.constraints.is_empty() {
        let lines: Vec<String> = ir
            .constraints
            .iter()
            .map(|c| match c.severity {
                pf_core::ConstraintSeverity::Required => format!("- (Pflicht) {}", c.text.trim()),
                pf_core::ConstraintSeverity::Recommended => {
                    format!("- (Empfohlen) {}", c.text.trim())
                }
            })
            .collect();
        push(&mut out, "CONSTRAINTS", &lines);
    }
    if !ir.assumptions.is_empty() {
        push(
            &mut out,
            "ANNAHMEN",
            &ir.assumptions
                .iter()
                .map(|s| format!("- {s}"))
                .collect::<Vec<_>>(),
        );
    }
    if let Some(role) = ir.role.as_deref().filter(|r| !r.trim().is_empty()) {
        push(&mut out, "ROLLE", &[role.trim().to_string()]);
    }
    if !ir.procedure.is_empty() {
        let lines: Vec<String> = ir
            .procedure
            .iter()
            .enumerate()
            .map(|(i, s)| format!("{}. {s}", i + 1))
            .collect();
        push(&mut out, "VORGEHEN", &lines);
    }
    if let Some(rs) = ir
        .reasoning_strategy
        .as_deref()
        .filter(|r| !r.trim().is_empty())
    {
        push(&mut out, "ARGUMENTATION", &[rs.trim().to_string()]);
    }
    if !ir.examples.is_empty() {
        let lines: Vec<String> = ir
            .examples
            .iter()
            .map(|e| {
                if e.output.trim().is_empty() {
                    format!("- Eingabe: {}", e.input.trim())
                } else {
                    format!(
                        "- Eingabe: {}\n  Ausgabe: {}",
                        e.input.trim(),
                        e.output.trim()
                    )
                }
            })
            .collect();
        push(&mut out, "BEISPIELE", &lines);
    }
    if !ir.output_contract.format.trim().is_empty()
        || !ir.output_contract.structure.is_empty()
        || !ir.output_contract.rules.is_empty()
    {
        let mut lines = Vec::new();
        if !ir.output_contract.format.trim().is_empty() {
            lines.push(format!("Format: {}", ir.output_contract.format.trim()));
        }
        for s in &ir.output_contract.structure {
            if !s.trim().is_empty() {
                lines.push(format!("- {}", s.trim()));
            }
        }
        for r in &ir.output_contract.rules {
            if !r.trim().is_empty() {
                lines.push(format!("Regel: {}", r.trim()));
            }
        }
        if let Some(ex) = ir
            .output_contract
            .example
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            lines.push(format!("Beispiel: {}", ex.trim()));
        }
        push(&mut out, "AUSGABEFORMAT", &lines);
    }
    if !ir.verification_requirements.is_empty() {
        push(
            &mut out,
            "VERIFIKATION",
            &ir.verification_requirements
                .iter()
                .map(|s| format!("- {s}"))
                .collect::<Vec<_>>(),
        );
    }
    join_string_lines(out)
}

/// Semantic Deduplication (konservativ): entfernt eine Zeile nur, wenn eine
/// andere Zeile alle ihre Tokens enthält (Subsumption beweisbar) ODER die
/// Zeile reine Diskurs-/Füll-Prosa ist. Keine technische Zeile wird
/// entfernt; keine Information wird geraten.
pub fn strategy_semantic(text: &str) -> (String, u64) {
    let lines: Vec<&str> = text.lines().collect();
    let mut removed = 0u64;
    let mut kept: Vec<&str> = Vec::new();
    let mut kept_tokens: Vec<std::collections::BTreeSet<String>> = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if is_protected_line(trimmed) {
            kept.push(trimmed);
            kept_tokens.push(token_set(trimmed));
            continue;
        }
        let toks = token_set(trimmed);
        if toks.len() < 2 {
            removed += 1;
            continue;
        }
        let subsumed = kept_tokens
            .iter()
            .any(|k| !k.is_empty() && toks.is_subset(k));
        if subsumed && toks.len() >= 3 {
            removed += 1;
            continue;
        }
        kept.push(trimmed);
        kept_tokens.push(toks);
    }
    (join_lines(kept), removed)
}

/// Combined: redundancy → instruction → semantic (deterministisch).
pub fn strategy_combined(text: &str) -> (String, u64) {
    let (t1, r1) = strategy_redundancy(text);
    let (t2, r2) = strategy_instruction(&t1);
    let (t3, r3) = strategy_semantic(&t2);
    (t3, r1 + r2 + r3)
}

/// Misst den Anteil erhaltener technischer Segmente (0..1): für jede
/// geschützte Zeile des Originals wird der Token-Erhalt im Kandidaten
/// gemittelt (Token-Mengen, case-insensitiv — keine Zeichen-Akrobatik).
pub fn technical_preservation(original: &str, candidate: &str) -> f64 {
    let cand_toks = token_set(candidate);
    let protected_lines: Vec<&str> = original
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && is_protected_line(l))
        .collect();
    if protected_lines.is_empty() {
        return 1.0;
    }
    let mut sum = 0.0;
    for line in &protected_lines {
        let toks = token_set(line);
        if toks.is_empty() {
            sum += 1.0;
            continue;
        }
        let matched = toks.iter().filter(|t| cand_toks.contains(*t)).count();
        sum += matched as f64 / toks.len() as f64;
    }
    sum / protected_lines.len() as f64
}

fn join_lines(lines: Vec<&str>) -> String {
    let owned: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    join_string_lines(owned)
}

fn join_string_lines(lines: Vec<String>) -> String {
    if lines.is_empty() {
        return String::new();
    }
    format!("{}\n", lines.join("\n"))
}

fn push(out: &mut Vec<String>, header: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push(String::new());
    }
    out.push(format!("{header}:"));
    out.extend(lines.iter().cloned());
}

#[cfg(test)]
mod tests {
    use super::*;
    use pf_core::ir::{Constraint, ConstraintSeverity, PromptIr};

    fn rich_ir() -> PromptIr {
        let mut ir = PromptIr::new("rid-opt", "Auditiere das Projekt");
        ir.objective = vec!["Alle Architekturrisiken identifizieren".to_string()];
        ir.context = vec!["Monorepo mit Rust-Core und Python-Layer".to_string()];
        ir.inputs = vec![pf_core::ir::InputSpec {
            name: "Projektpfad".to_string(),
            description: "Lokaler Repository-Pfad".to_string(),
        }];
        ir.constraints = vec![Constraint {
            text: "Nur Befunde mit Repository-Evidenz melden".to_string(),
            severity: ConstraintSeverity::Required,
        }];
        ir.assumptions = vec!["Lokale Entwicklung auf macOS".to_string()];
        ir.procedure = vec![
            "Repository-Struktur inspizieren".to_string(),
            "Abhängigkeiten prüfen".to_string(),
        ];
        ir.output_contract.format = "markdown".to_string();
        ir.output_contract.structure = vec!["Bericht mit Befunden".to_string()];
        ir.output_contract.rules = vec!["Keine Quellen ohne Beleg".to_string()];
        ir.verification_requirements = vec!["Jede Empfehlung begründen".to_string()];
        ir
    }

    #[test]
    fn protected_lines_detected() {
        assert!(is_protected_line("```rust"));
        assert!(is_protected_line("    let x = 1;"));
        assert!(is_protected_line("curl https://example.com/api"));
        assert!(is_protected_line("cd /Users/anton/repo"));
        assert!(is_protected_line("version 1.2.3 verwenden"));
        assert!(!is_protected_line(
            "Bitte prüfe die Architektur sorgfältig."
        ));
    }

    #[test]
    fn redundancy_removes_filler_and_subsumed() {
        let text = "Prüfe die Architektur sorgfältig.\nPrüfe die Architektur.\ncurl https://example.com/api\ncurl https://example.com/api\n";
        let (out, removed) = strategy_redundancy(text);
        assert!(removed >= 2);
        // „Prüfe die Architektur." ist Token-Teilmenge der längeren Zeile.
        assert!(!out.contains("Prüfe die Architektur.\n"));
        // Exakte technische Duplikate fallen, aber mindestens eine URL-Zeile bleibt.
        assert_eq!(out.matches("curl https://example.com/api").count(), 1);
    }

    #[test]
    fn instruction_compresses_hedges_but_keeps_meaning() {
        let text = "Please carefully inspect the repository and make sure that you identify any inconsistencies.\ncd /var/repo\n";
        let (out, removed) = strategy_instruction(text);
        assert!(removed >= 1);
        let lower = out.to_ascii_lowercase();
        assert!(lower.contains("inspect the repository"));
        assert!(lower.contains("identify any inconsistencies"));
        assert!(!lower.contains("please carefully"));
        // Technische Zeile unverändert.
        assert!(out.contains("cd /var/repo"));
    }

    #[test]
    fn structural_contains_all_mandatory_atoms_verbatim() {
        let ir = rich_ir();
        let out = strategy_structural(&ir);
        for atom in [
            "Alle Architekturrisiken identifizieren",
            "Nur Befunde mit Repository-Evidenz melden",
            "Repository-Struktur inspizieren",
            "Bericht mit Befunden",
            "Keine Quellen ohne Beleg",
            "Jede Empfehlung begründen",
            "Lokale Entwicklung auf macOS",
            "Projektpfad: Lokaler Repository-Pfad",
            "Monorepo mit Rust-Core und Python-Layer",
        ] {
            assert!(out.contains(atom), "fehlt: {atom}");
        }
        assert!(out.contains("AUFGABE:"));
        assert!(out.contains("VORGEHEN:"));
        assert!(out.contains("AUSGABEFORMAT:"));
    }

    #[test]
    fn semantic_dedup_is_conservative() {
        let text = "Preserve the existing API and backward compatibility.\nPreserve the existing API.\ncd /tmp/x\n";
        let (out, removed) = strategy_semantic(text);
        // „Preserve the existing API." ist Token-Teilmenge der längeren Zeile.
        assert!(removed >= 1);
        assert!(!out.contains("Preserve the existing API."));
        assert!(out.contains("Preserve the existing API and backward compatibility."));
        assert!(out.contains("cd /tmp/x"));
    }

    #[test]
    fn combined_reduces_more() {
        let text = "Please carefully inspect the repository and make sure that you identify inconsistencies.\nThe repository should be inspected and inconsistencies should be identified.\nNur Befunde mit Repository-Evidenz melden.\n";
        let (out, removed) = strategy_combined(text);
        assert!(removed >= 1);
        assert!(!out.is_empty());
    }

    #[test]
    fn technical_preservation_scores_kept_segments() {
        let orig = "Bitte prüfe die API.\ncurl -X GET https://api.example.com/v1/users?limit=10\ncd /app/src && cargo test\n";
        let cand = "Prüfe die API.\ncurl -X GET https://api.example.com/v1/users?limit=10\ncd /app/src && cargo test\n";
        let score = technical_preservation(orig, cand);
        assert!(score > 0.99, "score={score}");
        // Technische Zeile entfernt → Score fällt klar unter 1.
        let bad = technical_preservation(orig, "Prüfe die API.\n");
        assert!(bad < 0.8, "bad={bad}");
    }
}
