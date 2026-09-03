//! Typisierte Fehlerfamilie für PromptForge.
//!
//! Jeder Fehler hat ein stabiles `kind` (Spec §19) — maschinenlesbar über
//! JSON/API und Exit-Codes der CLI. Secrets erscheinen nie in `message`.

use serde::Serialize;

/// Stabile Fehlerkategorien (Spec §19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Configuration,
    Provider,
    Authentication,
    Model,
    Timeout,
    Tokenization,
    Optimization,
    Verification,
    Persistence,
    Bridge,
    Serialization,
    Json,
    Io,
    InvalidInput,
}

impl ErrorKind {
    /// CLI-Exit-Code (Spec §9/§19): 0 ok, 2 usage, 3 config, 4 LLM,
    /// 5 pipeline/verify, 6 persistence, 7 infra.
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorKind::InvalidInput => 2,
            ErrorKind::Configuration => 3,
            ErrorKind::Provider
            | ErrorKind::Authentication
            | ErrorKind::Model
            | ErrorKind::Timeout => 4,
            ErrorKind::Tokenization | ErrorKind::Optimization | ErrorKind::Verification => 5,
            ErrorKind::Persistence => 6,
            ErrorKind::Bridge | ErrorKind::Serialization | ErrorKind::Json | ErrorKind::Io => 7,
        }
    }

    /// Ob ein Retry sinnvoll ist (z. B. durch die Pipeline-/Service-Schicht).
    pub fn retryable(self) -> bool {
        matches!(
            self,
            ErrorKind::Provider | ErrorKind::Timeout | ErrorKind::Model | ErrorKind::Bridge
        )
    }
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ErrorKind::Configuration => "configuration",
            ErrorKind::Provider => "provider",
            ErrorKind::Authentication => "authentication",
            ErrorKind::Model => "model",
            ErrorKind::Timeout => "timeout",
            ErrorKind::Tokenization => "tokenization",
            ErrorKind::Optimization => "optimization",
            ErrorKind::Verification => "verification",
            ErrorKind::Persistence => "persistence",
            ErrorKind::Bridge => "bridge",
            ErrorKind::Serialization => "serialization",
            ErrorKind::Json => "json",
            ErrorKind::Io => "io",
            ErrorKind::InvalidInput => "invalid_input",
        };
        f.write_str(s)
    }
}

/// Zentraler PromptForge-Fehler.
#[derive(Debug, Clone)]
pub struct PfError {
    pub kind: ErrorKind,
    pub message: String,
}

impl PfError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn invalid_input(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidInput, msg)
    }
}

impl std::fmt::Display for PfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for PfError {}

/// JSON-Darstellung für API/Logs; `retryable` wird mitgeliefert.
impl Serialize for PfError {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("PfError", 3)?;
        s.serialize_field("kind", &self.kind)?;
        s.serialize_field("message", &self.message)?;
        s.serialize_field("retryable", &self.kind.retryable())?;
        s.end()
    }
}

pub type Result<T> = std::result::Result<T, PfError>;

pub fn err(kind: ErrorKind, msg: impl Into<String>) -> PfError {
    PfError::new(kind, msg)
}

impl From<std::io::Error> for PfError {
    fn from(e: std::io::Error) -> Self {
        Self::new(ErrorKind::Io, e.to_string())
    }
}

impl From<serde_json::Error> for PfError {
    fn from(e: serde_json::Error) -> Self {
        Self::new(ErrorKind::Json, format!("JSON-Fehler: {e}"))
    }
}

impl From<toml::de::Error> for PfError {
    fn from(e: toml::de::Error) -> Self {
        Self::new(ErrorKind::Configuration, format!("Config-Fehler: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kind_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&ErrorKind::Verification).unwrap(),
            "\"verification\""
        );
        assert_eq!(ErrorKind::Authentication.exit_code(), 4);
        assert!(ErrorKind::Timeout.retryable());
        assert!(!ErrorKind::InvalidInput.retryable());
    }

    #[test]
    fn error_json_roundtrip_has_expected_fields() {
        let e = PfError::new(ErrorKind::Provider, "kaputt");
        let v: serde_json::Value = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "provider");
        assert_eq!(v["message"], "kaputt");
        assert_eq!(v["retryable"], true);
    }
}
