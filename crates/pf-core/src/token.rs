//! Token-Accounting (Spec §8).
//!
//! Tokenisierung ist über das Trait `Tokenizer` abstrahiert. v0.1 liefert
//! einen deterministischen Heuristik-Tokenizer (als Schätzung gekennzeichnet);
//! exakte, modellspezifische Tokenizer können später als weitere
//! Implementierungen ergänzt werden (auch Python-seitig über die Bridge).

use serde::{Deserialize, Serialize};

/// Abstraktion für Tokenzähler.
pub trait Tokenizer: Send + Sync {
    fn count(&self, text: &str) -> u64;
    /// Name/Modell-Bezug (z. B. "heuristic", "tiktoken/cl100k_base").
    fn name(&self) -> &'static str;
    /// `true`, wenn es sich um eine Schätzung handelt (Spec §8: transparent).
    fn is_estimate(&self) -> bool;
}

/// Deterministischer Schätzer: Wörter + Interpunktionsläufe + langes Wort.
///
/// Formel (dokumentiert, bewusst simpel):
/// `tokens = words + punct_runs + long_words`, wobei Wörter per
/// Whitespace-Split und Interpunktionsläufe als zusammenhängende Nicht-
/// alphanumerische Sequenzen gezählt werden.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeuristicTokenizer;

impl Tokenizer for HeuristicTokenizer {
    fn count(&self, text: &str) -> u64 {
        #[derive(Clone, Copy, PartialEq)]
        enum Kind {
            Start,
            Word,
            Punct,
            Space,
        }
        fn kind_of(ch: char) -> Kind {
            if ch.is_alphanumeric() {
                Kind::Word
            } else if ch.is_whitespace() {
                Kind::Space
            } else {
                Kind::Punct
            }
        }

        let mut words: u64 = 0;
        let mut punct_runs: u64 = 0;
        let mut long_words: u64 = 0;
        let mut pending_word_len: usize = 0;
        let mut prev = Kind::Start;

        let finish_word = |words: &mut u64, long_words: &mut u64, pending: &mut usize| {
            if *pending > 0 {
                *words += 1;
                if *pending > 12 {
                    *long_words += 1;
                }
                *pending = 0;
            }
        };

        for ch in text.chars() {
            let k = kind_of(ch);
            match k {
                Kind::Word => {
                    if prev != Kind::Word {
                        pending_word_len = 0;
                    }
                    pending_word_len += 1;
                }
                Kind::Punct => {
                    finish_word(&mut words, &mut long_words, &mut pending_word_len);
                    if prev != Kind::Punct {
                        punct_runs += 1;
                    }
                }
                Kind::Space => {
                    finish_word(&mut words, &mut long_words, &mut pending_word_len);
                }
                Kind::Start => {}
            }
            prev = k;
        }
        finish_word(&mut words, &mut long_words, &mut pending_word_len);
        words + punct_runs + long_words
    }

    fn name(&self) -> &'static str {
        "heuristic"
    }

    fn is_estimate(&self) -> bool {
        true
    }
}

/// Token-Statistik je Pipeline-Stufe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct StageTokens {
    pub stage: String,
    pub tokens: u64,
    /// true = Schätzung (heuristisch), false = exakt.
    pub estimate: bool,
}

/// Token-Report für die Gesamt-Pipeline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenReport {
    pub original: u64,
    pub generated: u64,
    pub optimized: u64,
    pub stages: Vec<StageTokens>,
    /// true = Zählungen sind Schätzungen (heuristischer Tokenizer).
    #[serde(default = "default_true")]
    pub estimate: bool,
}

fn default_true() -> bool {
    true
}

impl Default for TokenReport {
    fn default() -> Self {
        Self {
            original: 0,
            generated: 0,
            optimized: 0,
            stages: Vec::new(),
            estimate: true,
        }
    }
}

impl TokenReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_original(&mut self, tokens: u64) {
        self.original = tokens;
    }

    pub fn set_generated(&mut self, tokens: u64) {
        self.generated = tokens;
    }

    pub fn set_optimized(&mut self, tokens: u64) {
        self.optimized = tokens;
    }

    pub fn push_stage(&mut self, stage: &str, tokens: u64, estimate: bool) {
        self.stages.push(StageTokens {
            stage: stage.to_string(),
            tokens,
            estimate,
        });
    }

    /// Reduktion in Prozent (0.0 wenn keine Basis).
    pub fn reduction_pct(&self) -> f64 {
        if self.generated == 0 {
            return 0.0;
        }
        ((self.generated as f64 - self.optimized as f64) / self.generated as f64 * 100.0).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_counts_words() {
        let t = HeuristicTokenizer;
        assert_eq!(t.count(""), 0);
        assert_eq!(t.count("one two three"), 3);
    }

    #[test]
    fn heuristic_counts_punct_and_long_words() {
        let t = HeuristicTokenizer;
        // "hello, world!" → words: hello, world (2) + punct_runs: ",", "!" (2)
        assert_eq!(t.count("hello, world!"), 4);
        // 13-Buchstaben-Wort → +1 long word
        assert_eq!(t.count("antidisestablishment"), 2);
    }

    #[test]
    fn heuristic_marked_as_estimate() {
        let t = HeuristicTokenizer;
        assert!(t.is_estimate());
        assert_eq!(t.name(), "heuristic");
    }

    #[test]
    fn report_reduction_pct() {
        let mut r = TokenReport::new();
        r.set_original(84);
        r.set_generated(2481);
        r.set_optimized(1127);
        let pct = r.reduction_pct();
        assert!((pct - 54.6).abs() < 0.1, "pct={pct}");
    }

    #[test]
    fn report_reduction_zero_when_no_generated() {
        let mut r = TokenReport::new();
        r.set_generated(0);
        r.set_optimized(0);
        assert_eq!(r.reduction_pct(), 0.0);
    }

    #[test]
    fn report_reduction_clamped_at_zero() {
        let mut r = TokenReport::new();
        r.set_generated(10);
        r.set_optimized(20); // Optimierung verschlechtert → kein negatives Delta
        assert_eq!(r.reduction_pct(), 0.0);
    }
}
