//! v0.2 Phase 2: Serializer-Layer (Design §4/§7/§9–§14).
//!
//! `CompilationResult` bleibt formatneutral (kennt KEINE Ausgabeformate).
//! Die Ausgabe erfolgt über:
//!
//! ```text
//! CompilationResult
//!        │
//!        ├── TextSerializer   → ausführbarer Prompt (optimized_prompt)
//!        ├── JsonSerializer   → CompilationResult-Envelope als JSON
//!        ├── YamlSerializer   → derselbe Envelope als YAML
//!        └── ToonSerializer   → derselbe Envelope als TOON (Spec v3.0)
//! ```
//!
//! text = ausführbarer Prompt; json/yaml/toon = strukturierter Envelope
//! (dasselbe Datenmodell). Serialisierung ist deterministisch, rein lokal
//! und löst keinerlei LLM-Aufruf aus.

use std::fmt;
use std::str::FromStr;

use crate::compilation::CompilationResult;
use crate::error::{ErrorKind, Result, err};

/// Zentrale Ausgabeformate (Spec §5/§9/§10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputFormat {
    /// Ausführbarer Prompt (optimierter Prompt; Default).
    Text,
    /// Strukturierter CompilationResult-Envelope als JSON.
    Json,
    /// Strukturierter CompilationResult-Envelope als YAML.
    Yaml,
    /// Strukturierter CompilationResult-Envelope als TOON.
    Toon,
}

impl OutputFormat {
    /// Kanonische, kleingeschriebene Werte.
    pub const ALL: [OutputFormat; 4] = [
        OutputFormat::Text,
        OutputFormat::Json,
        OutputFormat::Yaml,
        OutputFormat::Toon,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            OutputFormat::Text => "text",
            OutputFormat::Json => "json",
            OutputFormat::Yaml => "yaml",
            OutputFormat::Toon => "toon",
        }
    }

    /// Ist das Format strukturiert (Envelope) statt ausführbarem Prompt?
    pub fn is_structured(self) -> bool {
        !matches!(self, OutputFormat::Text)
    }

    /// Verzeichnis-/Datei-Endung für Artefakte (ohne Punkt).
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Text => "txt",
            OutputFormat::Json => "json",
            OutputFormat::Yaml => "yaml",
            OutputFormat::Toon => "toon",
        }
    }

    /// Parsen mit toleranter Groß-/Kleinschreibung (CLI-freundlich).
    /// Unbekannte Werte → Fehler (kein stiller Fallback auf text).
    pub fn parse_loose(s: &str) -> Result<Self> {
        let lower = s.trim().to_ascii_lowercase();
        match lower.as_str() {
            "text" | "txt" => Ok(OutputFormat::Text),
            "json" => Ok(OutputFormat::Json),
            "yaml" | "yml" => Ok(OutputFormat::Yaml),
            "toon" => Ok(OutputFormat::Toon),
            _ => Err(err(
                ErrorKind::InvalidInput,
                format!("unbekanntes Ausgabeformat: {s:?} (erwartet: text | json | yaml | toon)"),
            )),
        }
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OutputFormat {
    type Err = crate::PfError;
    fn from_str(s: &str) -> Result<Self> {
        OutputFormat::parse_loose(s)
    }
}

/// Serializer-Interface (Design §4). Implementierungen sind zustandslos.
pub trait PromptSerializer {
    /// Format, das dieser Serializer erzeugt.
    fn format(&self) -> OutputFormat;

    /// Serialisiert ein `CompilationResult` deterministisch in einen String.
    fn serialize(&self, result: &CompilationResult) -> Result<String>;
}

/// Serialisiert den strukturierten Envelope eines `CompilationResult`.
fn envelope_of(result: &CompilationResult) -> serde_json::Value {
    result.envelope_json()
}

/// TextSerializer: liefert den ausführbaren Prompt (optimized_prompt).
/// Kein Dump des CompilationResult, keine zweite LLM-Generation.
pub struct TextSerializer;

impl PromptSerializer for TextSerializer {
    fn format(&self) -> OutputFormat {
        OutputFormat::Text
    }

    fn serialize(&self, result: &CompilationResult) -> Result<String> {
        Ok(result.optimized_prompt.clone())
    }
}

/// JsonSerializer: deterministischer Envelope als hübsches JSON.
pub struct JsonSerializer;

impl PromptSerializer for JsonSerializer {
    fn format(&self) -> OutputFormat {
        OutputFormat::Json
    }

    fn serialize(&self, result: &CompilationResult) -> Result<String> {
        to_json_string(&envelope_of(result))
    }
}

/// YamlSerializer: derselbe Envelope als YAML (via `serde_norway`,
/// gepflegter Fork der serde_yaml-0.9-API).
pub struct YamlSerializer;

impl PromptSerializer for YamlSerializer {
    fn format(&self) -> OutputFormat {
        OutputFormat::Yaml
    }

    fn serialize(&self, result: &CompilationResult) -> Result<String> {
        to_yaml_string(&envelope_of(result))
    }
}

/// ToonSerializer: derselbe Envelope als TOON (offizielle Rust-Implementierung
/// `toon-format` 0.5, TOON-Spec v3.0, MIT).
pub struct ToonSerializer;

impl PromptSerializer for ToonSerializer {
    fn format(&self) -> OutputFormat {
        OutputFormat::Toon
    }

    fn serialize(&self, result: &CompilationResult) -> Result<String> {
        to_toon_string(&envelope_of(result))
    }
}

/// Liefert den passenden Serializer für ein Format.
pub fn serializer_for(format: OutputFormat) -> Box<dyn PromptSerializer> {
    match format {
        OutputFormat::Text => Box::new(TextSerializer),
        OutputFormat::Json => Box::new(JsonSerializer),
        OutputFormat::Yaml => Box::new(YamlSerializer),
        OutputFormat::Toon => Box::new(ToonSerializer),
    }
}

// --- Strukturierte Render-Funktionen (auch für CLI-Dokumente mit `saved`) ---

/// JSON-Darstellung eines strukturierten Dokuments (deterministisch,
/// pretty). Gleiches Datenmodell wie YAML/TOON.
pub fn to_json_string(doc: &serde_json::Value) -> Result<String> {
    serde_json::to_string_pretty(doc)
        .map(|s| {
            // serde_json pretty hängt kein abschließendes Newline an; für
            // Dateien/CLI konsistent ein Newline ergänzen.
            format!("{s}\n")
        })
        .map_err(|e| {
            err(
                ErrorKind::Serialization,
                format!("JSON-Serialisierung: {e}"),
            )
        })
}

/// YAML-Darstellung eines strukturierten Dokuments (serde_norway 0.9).
pub fn to_yaml_string(doc: &serde_json::Value) -> Result<String> {
    serde_norway::to_string(doc).map_err(|e| {
        err(
            ErrorKind::Serialization,
            format!("YAML-Serialisierung: {e}"),
        )
    })
}

/// TOON-Darstellung eines strukturierten Dokuments (toon-format 0.5,
/// offizielle Rust-Implementierung, TOON-Spec v3.0).
pub fn to_toon_string(doc: &serde_json::Value) -> Result<String> {
    toon_format::encode_default(doc).map_err(|e| {
        err(
            ErrorKind::Serialization,
            format!("TOON-Serialisierung: {e}"),
        )
    })
}

/// YAML → serde_json::Value (Roundtrip-/Äquivalenz-Tests).
pub fn yaml_to_json(s: &str) -> Result<serde_json::Value> {
    serde_norway::from_str::<serde_json::Value>(s)
        .map_err(|e| err(ErrorKind::Serialization, format!("YAML-Parsing: {e}")))
}

/// TOON → serde_json::Value (Roundtrip-/Conformance-Tests; Decoder der
/// offiziellen Rust-Crate).
pub fn toon_to_json(s: &str) -> Result<serde_json::Value> {
    toon_format::decode_default::<serde_json::Value>(s)
        .map_err(|e| err(ErrorKind::Serialization, format!("TOON-Parsing: {e}")))
}

/// Serialisiert ein strukturiertes Dokument im gewünschten Format.
/// `text` ist hier nicht zulässig (eigenständig behandelt).
pub fn render_structured(format: OutputFormat, doc: &serde_json::Value) -> Result<String> {
    match format {
        OutputFormat::Text => Err(err(
            ErrorKind::InvalidInput,
            "render_structured unterstützt nur json/yaml/toon",
        )),
        OutputFormat::Json => to_json_string(doc),
        OutputFormat::Yaml => to_yaml_string(doc),
        OutputFormat::Toon => to_toon_string(doc),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compilation::CompilationResult;
    use crate::ir::{Constraint, ConstraintSeverity};
    use crate::token::TokenReport;
    use crate::verify::{Verdict, VerificationReport};

    /// Deterministisches Golden-Fixture — kein LLM involviert.
    pub(crate) fn golden_result() -> CompilationResult {
        let mut ir = crate::ir::PromptIr::new("rid-golden", "Auditiere das Projekt");
        ir.objective = vec!["Schwachstellen identifizieren".to_string()];
        ir.constraints = vec![Constraint {
            text: "Keine erfundenen Befunde; Evidenz aus dem Repository zitieren".to_string(),
            severity: ConstraintSeverity::Required,
        }];
        ir.procedure = vec![
            "Repository-Struktur prüfen".to_string(),
            "Abhängigkeiten prüfen".to_string(),
        ];
        ir.output_contract.format = "markdown".to_string();
        ir.output_contract.structure = vec![
            "Zusammenfassung".to_string(),
            "Befunde mit Schweregrad".to_string(),
        ];
        ir.analysis = Some(crate::ir::IntentAnalysis {
            task_type: Some("audit".to_string()),
            language: Some("de".to_string()),
            ..Default::default()
        });
        ir.metadata.tags = vec!["test".to_string()];
        let mut tr = TokenReport::new();
        tr.set_original(4);
        tr.set_generated(120);
        tr.set_optimized(90);
        let verification = VerificationReport {
            semantic_preservation: 0.98,
            constraints_preserved: true,
            output_contract_preserved: true,
            objective_preserved: true,
            instructions_preserved: true,
            verification_requirements_preserved: true,
            verdict: Some(Verdict::Pass),
            attempts: 1,
            ..Default::default()
        };
        CompilationResult {
            input: "Auditiere das Projekt".to_string(),
            request_id: "rid-golden".to_string(),
            llm_used: false,
            stages: vec![
                "architect".to_string(),
                "expand".to_string(),
                "optimize".to_string(),
                "verify".to_string(),
            ],
            prompt_ir: ir,
            expanded_prompt: "## Aufgabe\nAuditiere das Projekt\n## Constraints\nKeine erfundenen Befunde; Evidenz aus dem Repository zitieren\n## Ausgabeformat\nZusammenfassung\nBefunde mit Schweregrad"
                .to_string(),
            optimized_prompt: "Du bist ein Senior-Software-Architekt.\n\n## Aufgabe\nAuditiere das Projekt\n\n## Constraints\nKeine erfundenen Befunde; Evidenz aus dem Repository zitieren\n\n## Ausgabeformat\n- Zusammenfassung\n- Befunde mit Schweregrad"
                .to_string(),
            token_report: tr,
            verification,
            metrics: crate::compilation::QualityMetrics {
                semantic_fidelity: 0.98,
                structural_validity: true,
                token_efficiency: 0.25,
            },
        }
    }

    #[test]
    fn output_format_parsing_and_display() {
        assert_eq!(
            OutputFormat::parse_loose("text").unwrap(),
            OutputFormat::Text
        );
        assert_eq!(
            OutputFormat::parse_loose("JSON").unwrap(),
            OutputFormat::Json
        );
        assert_eq!(
            OutputFormat::parse_loose(" Yaml ").unwrap(),
            OutputFormat::Yaml
        );
        assert_eq!(
            OutputFormat::parse_loose("toon").unwrap(),
            OutputFormat::Toon
        );
        assert_eq!(
            OutputFormat::parse_loose("yml").unwrap(),
            OutputFormat::Yaml
        );
        assert_eq!(
            OutputFormat::parse_loose("TXT").unwrap(),
            OutputFormat::Text
        );
        assert!(OutputFormat::parse_loose("banana").is_err());
        assert_eq!(OutputFormat::Text.to_string(), "text");
        assert_eq!(OutputFormat::Text.as_str(), "text");
        assert!(!OutputFormat::Text.is_structured());
        assert!(OutputFormat::Toon.is_structured());
        assert_eq!(OutputFormat::Toon.extension(), "toon");
        for f in OutputFormat::ALL {
            assert_eq!(OutputFormat::parse_loose(f.as_str()).unwrap(), f);
        }
    }

    #[test]
    fn text_serializer_returns_executable_prompt() {
        let result = golden_result();
        let out = TextSerializer.serialize(&result).unwrap();
        assert_eq!(out, result.optimized_prompt);
        assert!(out.starts_with("Du bist ein Senior-Software-Architekt."));
        assert!(!out.contains("request_id"));
        assert!(!out.contains("metrics"));
    }

    #[test]
    fn json_serializer_deterministic() {
        let result = golden_result();
        let a = JsonSerializer.serialize(&result).unwrap();
        let b = JsonSerializer.serialize(&result).unwrap();
        assert_eq!(a, b, "JSON-Serialisierung muss deterministisch sein");
        let v: serde_json::Value = serde_json::from_str(&a).unwrap();
        assert_eq!(v["input"], "Auditiere das Projekt");
        assert!(v.get("prompt_ir").is_some());
        assert!(v.get("ir").is_some()); // Legacy-Alias
        assert!(v.get("metrics").is_some());
        assert_eq!(v["verification"]["verdict"], "pass");
        // Kanonische Struktur: gleiches Datenmodell wie YAML/TOON.
        assert!(v.get("expanded_prompt").is_some());
        assert!(v.get("long_prompt").is_some());
    }

    #[test]
    fn yaml_serializer_roundtrip() {
        let result = golden_result();
        let yaml = YamlSerializer.serialize(&result).unwrap();
        assert!(!yaml.trim().is_empty());
        // Roundtrip YAML → CompilationResult (semantisch äquivalent).
        let back: CompilationResult = serde_norway::from_str(&yaml).unwrap();
        assert_eq!(back, result);
        // YAML → JSON-Wert == Envelope (gleiches Datenmodell wie JSON).
        let yaml_val = yaml_to_json(&yaml).unwrap();
        let json_val =
            serde_json::from_str::<serde_json::Value>(&JsonSerializer.serialize(&result).unwrap())
                .unwrap();
        assert_eq!(yaml_val, json_val);
    }

    #[test]
    fn toon_serializer_roundtrip() {
        let result = golden_result();
        let toon = ToonSerializer.serialize(&result).unwrap();
        assert!(!toon.trim().is_empty());
        // Roundtrip TOON → JSON-Datenmodell über den offiziellen Decoder.
        let decoded = toon_to_json(&toon).unwrap();
        let json_val =
            serde_json::from_str::<serde_json::Value>(&JsonSerializer.serialize(&result).unwrap())
                .unwrap();
        assert_eq!(decoded, json_val, "TOON muss dasselbe Datenmodell ergeben");
        // Determinismus.
        assert_eq!(toon, ToonSerializer.serialize(&result).unwrap());
    }

    #[test]
    fn yaml_and_toon_deterministic() {
        let result = golden_result();
        assert_eq!(
            YamlSerializer.serialize(&result).unwrap(),
            YamlSerializer.serialize(&result).unwrap()
        );
        assert_eq!(
            ToonSerializer.serialize(&result).unwrap(),
            ToonSerializer.serialize(&result).unwrap()
        );
    }

    #[test]
    fn serializers_handle_unicode_multiline_and_special_chars() {
        let mut result = golden_result();
        result.input =
            "Prüfe „Anführungszeichen“, Zeilenumbrüche\nund Unicode: 中文 / emoji 🚀".to_string();
        result.optimized_prompt =
            "Zeile 1\nZeile \"mit\" Quotes\nTab\tund 中文\nBackslash \\ und `code`".to_string();
        result.expanded_prompt = result.optimized_prompt.clone();
        let mut ir = result.prompt_ir.clone();
        ir.objective
            .push("Überprüfe Sonderzeichen: \"quotes\", 'single', \\, \n, 中文, 🚀".to_string());
        result.prompt_ir = ir;
        // Text: unverändert (ausführbarer Prompt).
        assert_eq!(
            TextSerializer.serialize(&result).unwrap(),
            result.optimized_prompt
        );
        // Strukturierte Formate: Roundtrips erhalten das Datenmodell.
        let yaml = YamlSerializer.serialize(&result).unwrap();
        let back: CompilationResult = serde_norway::from_str(&yaml).unwrap();
        assert_eq!(back, result);
        let toon = ToonSerializer.serialize(&result).unwrap();
        let decoded = toon_to_json(&toon).unwrap();
        let json_val =
            serde_json::from_str::<serde_json::Value>(&JsonSerializer.serialize(&result).unwrap())
                .unwrap();
        assert_eq!(decoded, json_val);
    }

    #[test]
    fn serializer_for_dispatch() {
        assert_eq!(
            serializer_for(OutputFormat::Text).format(),
            OutputFormat::Text
        );
        assert_eq!(
            serializer_for(OutputFormat::Json).format(),
            OutputFormat::Json
        );
        assert_eq!(
            serializer_for(OutputFormat::Yaml).format(),
            OutputFormat::Yaml
        );
        assert_eq!(
            serializer_for(OutputFormat::Toon).format(),
            OutputFormat::Toon
        );
    }

    #[test]
    fn json_value_roundtrip_envelope() {
        // JSON → serde_json::Value → CompilationResult (Golden-Fixture).
        let result = golden_result();
        let json = JsonSerializer.serialize(&result).unwrap();
        let back: CompilationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, result);
    }

    #[test]
    fn render_structured_rejects_text() {
        assert!(render_structured(OutputFormat::Text, &serde_json::json!({})).is_err());
        assert!(render_structured(OutputFormat::Json, &serde_json::json!({"a": 1})).is_ok());
        assert!(render_structured(OutputFormat::Toon, &serde_json::json!({"a": 1})).is_ok());
    }
}
