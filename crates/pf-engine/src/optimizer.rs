//! Deterministische Optimizer-Passes (Spec §6).
//!
//! Optimierung ist eine nachvollziehbare Pass-Pipeline, kein Black Box:
//! jeder Pass protokolliert, was er getan hat (`OptimizerEvent`).

/// Was ein Pass entschieden/getan hat — wird im `OptimizerEvent` sichtbar.
#[derive(Debug, Clone, PartialEq)]
pub enum PassAction {
    /// Anzahl entfernter exakter Duplikat-Zeilen.
    RemovedDuplicateLines(usize),
    /// Anzahl entfernter nahezu identischer Zeilen.
    RemovedNearDuplicateLines(usize),
    /// Anzahl kollabierter Leerzeilen-Blöcke.
    CollapsedBlankBlocks(usize),
    /// Anzahl wieder eingefügter fehlender Pflicht-Atome (Guard-Pass).
    ReinsertedAtoms(usize),
    /// LLM-Pass (extern) — Referenz auf die Stufe.
    LlmPass { stage: String, token_count: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct OptimizerEvent {
    pub pass: String,
    pub action: PassAction,
}

/// Pass 1: Whitespace-Normalisierung (Zeilen trimmen — eingerückter Code mit
/// 4+ Leerzeichen/Tab bleibt erhalten —, Leerzeilen-Blöcke kollabieren,
/// genau eine abschließende Newline).
pub fn normalize_whitespace(text: &str) -> (String, OptimizerEvent) {
    let mut collapsed = 0usize;
    let mut out_lines: Vec<String> = Vec::new();
    let mut blank_run = 0usize;
    for raw in text.lines() {
        let line = if raw.starts_with("    ") || raw.starts_with('\t') {
            raw.trim_end().to_string()
        } else {
            raw.trim().to_string()
        };
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                collapsed += 1;
            }
        } else {
            if !out_lines.is_empty() && blank_run > 0 {
                out_lines.push(String::new());
            }
            blank_run = 0;
            out_lines.push(line);
        }
    }
    let normalized = if out_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", out_lines.join("\n"))
    };
    (
        normalized,
        OptimizerEvent {
            pass: "normalize_whitespace".to_string(),
            action: PassAction::CollapsedBlankBlocks(collapsed),
        },
    )
}

/// Pass 2: exakte Duplikat-Zeilen entfernen (erste Vorkommnis bleibt).
pub fn dedupe_exact_lines(text: &str) -> (String, OptimizerEvent) {
    let mut seen = std::collections::HashSet::new();
    let mut removed = 0usize;
    let mut out = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let key = trimmed.to_ascii_lowercase();
        if trimmed.is_empty() || seen.insert(key) {
            out.push_str(line);
            out.push('\n');
        } else {
            removed += 1;
        }
    }
    (
        out,
        OptimizerEvent {
            pass: "dedupe_exact_lines".to_string(),
            action: PassAction::RemovedDuplicateLines(removed),
        },
    )
}

/// Pass 3: nahezu identische Zeilen entfernen (normalisierte Token-Menge
/// mit Jaccard ≥ `threshold` gilt als Duplikat).
pub fn dedupe_near_lines(text: &str, threshold: f64) -> (String, OptimizerEvent) {
    let mut kept_tokens: Vec<std::collections::BTreeSet<String>> = Vec::new();
    let mut removed = 0usize;
    let mut out = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.chars().count() < 12 {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let tokens = token_set(trimmed);
        let is_dup = kept_tokens.iter().any(|k| jaccard(k, &tokens) >= threshold);
        if is_dup {
            removed += 1;
        } else {
            kept_tokens.push(tokens);
            out.push_str(line);
            out.push('\n');
        }
    }
    (
        out,
        OptimizerEvent {
            pass: "dedupe_near_lines".to_string(),
            action: PassAction::RemovedNearDuplicateLines(removed),
        },
    )
}

/// Pass 4 (Guard): fehlende Pflicht-Atome ans Ende anhängen, damit
/// Constraints/Output-Contract strukturell erhalten bleiben.
pub fn reinsert_missing_atoms(text: &str, atoms: &[String]) -> (String, OptimizerEvent) {
    let haystack_lower = text.to_ascii_lowercase();
    let mut missing: Vec<&String> = Vec::new();
    for atom in atoms {
        if atom.trim().is_empty() {
            continue;
        }
        let normalized_atom = normalize_for_containment(atom);
        // Exakte Teilstring-Prüfung reicht für Pflicht-Atome.
        if !haystack_lower.contains(&normalized_atom) && !text.contains(atom.trim()) {
            missing.push(atom);
        }
    }
    if missing.is_empty() {
        return (
            text.to_string(),
            OptimizerEvent {
                pass: "reinsert_missing_atoms".to_string(),
                action: PassAction::ReinsertedAtoms(0),
            },
        );
    }
    let mut out = text.trim_end().to_string();
    out.push_str("\n\n## Erhaltungspflicht\n");
    for atom in &missing {
        out.push_str("- ");
        out.push_str(atom.trim());
        out.push('\n');
    }
    out.push('\n');
    (
        out,
        OptimizerEvent {
            pass: "reinsert_missing_atoms".to_string(),
            action: PassAction::ReinsertedAtoms(missing.len()),
        },
    )
}

/// Standard-Passkette (ohne LLM): Whitespace → exakte → nahe Duplikate.
pub fn deterministic_pass_chain(text: &str) -> (String, Vec<OptimizerEvent>) {
    let mut events = Vec::new();
    let (t1, e1) = normalize_whitespace(text);
    events.push(e1);
    let (t2, e2) = dedupe_exact_lines(&t1);
    events.push(e2);
    let (t3, e3) = dedupe_near_lines(&t2, 0.85);
    events.push(e3);
    (t3, events)
}

// --- Hilfsfunktionen ---

/// Häufige Funktionswörter (de/en), die für Semantik-Vergleiche Rauschen sind.
const STOPWORDS: &[&str] = &[
    "die", "der", "das", "den", "dem", "des", "und", "oder", "aber", "ein", "eine", "einer",
    "eines", "nicht", "ist", "sind", "auch", "mit", "von", "zu", "an", "auf", "für", "im", "the",
    "of", "and", "to", "for", "with", "in", "on", "at", "a", "an", "is", "are", "be", "as", "or",
    "but", "it", "this", "that", "die", "keine",
];

/// Lowercase-Token-Menge (alphanumerische Wörter ≥ 2 Zeichen, ohne
/// Funktionswörter).
pub fn token_set(text: &str) -> std::collections::BTreeSet<String> {
    text.to_ascii_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2 && !STOPWORDS.contains(w))
        .map(str::to_string)
        .collect()
}

/// Kompakte, normalisierte Teilstring-Form eines Pflicht-Atoms.
pub fn normalize_for_containment(atom: &str) -> String {
    atom.to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn jaccard(a: &std::collections::BTreeSet<String>, b: &std::collections::BTreeSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let inter = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        1.0
    } else {
        inter as f64 / union as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapses_blank_blocks_and_trims() {
        let input = "Zeile1\n\n\n\nZeile2\n   Zeile3  \n\n";
        let (out, ev) = normalize_whitespace(input);
        assert_eq!(out, "Zeile1\n\nZeile2\nZeile3\n");
        assert!(matches!(ev.action, PassAction::CollapsedBlankBlocks(n) if n == 2));
    }

    #[test]
    fn dedupe_exact_removes_second_occurrence() {
        let input = "a\nb\na\nb\n";
        let (out, ev) = dedupe_exact_lines(input);
        assert_eq!(out, "a\nb\n");
        assert!(matches!(ev.action, PassAction::RemovedDuplicateLines(2)));
    }

    #[test]
    fn dedupe_near_removes_similar_lines() {
        let input = "Wir müssen die Methode vergleichen und bewerten.\nWir müssen die Methode vergleichen und bewerten!\nAndere Zeile ganz anderer Art.\n";
        let (out, ev) = dedupe_near_lines(input, 0.85);
        assert_eq!(out.lines().count(), 2);
        assert!(matches!(
            ev.action,
            PassAction::RemovedNearDuplicateLines(1)
        ));
    }

    #[test]
    fn reinsert_adds_only_missing_atoms() {
        let text = "Bestehender Text mit Methode.";
        let atoms = vec![
            "Methode".to_string(),
            "Nur Quellen aus peer-reviewten Journals".to_string(),
        ];
        let (out, ev) = reinsert_missing_atoms(text, &atoms);
        assert!(out.contains("## Erhaltungspflicht"));
        assert!(matches!(ev.action, PassAction::ReinsertedAtoms(1)));
        let (out2, ev2) = reinsert_missing_atoms(&out, &atoms);
        assert!(matches!(ev2.action, PassAction::ReinsertedAtoms(0)));
        assert_eq!(out, out2);
    }

    #[test]
    fn chain_produces_events_in_order() {
        let input = "x\nx\n\n\n\ny\n";
        let (out, events) = deterministic_pass_chain(input);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].pass, "normalize_whitespace");
        // x, Leerzeile, y — das zweite x wurde entfernt.
        assert_eq!(out.lines().count(), 3);
    }

    #[test]
    fn token_set_lowercases_and_filters() {
        let s = token_set("Nur Quellen, die peer-reviewt sind!");
        assert!(s.contains("quellen"));
        assert!(!s.contains("die"));
        assert!(!s.contains("!"));
    }
}
