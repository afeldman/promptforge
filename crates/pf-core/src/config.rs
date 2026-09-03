//! Konfiguration (Spec §15): Defaults < `config/config.toml` < Environment.
//!
//! Kernvariablen: `LLM_ENDPOINT`, `LLM_KEY`, `LLM_MODEL`; dazu `PF_*`-Werte.
//! Secrets werden niemals über `Debug`/Logs ausgegeben (redigiert).

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{ErrorKind, Result, err};
use crate::redact::mask_secret;

/// Provider-Auswahl (Spec: keine Provider-Architektur im Rust-Core — die
/// Auswahl steuert nur, welche Python-Implementierung die Bridge nutzt).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    /// Endpoint/Model vorhanden → any_llm, sonst kein LLM (deterministisch).
    #[default]
    Auto,
    AnyLlm,
    /// Deterministischer Fake-LLM (Tests/Demo, kein Netz).
    Mock,
    /// Kein LLM-Aufruf (deterministische Pipeline).
    None,
}

impl ProviderKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Some(Self::Auto),
            "any_llm" | "anyllm" | "any-llm" => Some(Self::AnyLlm),
            "mock" => Some(Self::Mock),
            "none" | "off" => Some(Self::None),
            _ => None,
        }
    }
}

/// LLM-Basiskonfiguration (Spec §3).
/// Bewusst ohne abgeleitetes `Debug` — Secrets werden redigiert (eigene Impl).
#[derive(Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LlmConfig {
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "default_timeout_s")]
    pub timeout_s: u64,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

fn default_timeout_s() -> u64 {
    120
}

impl LlmConfig {
    /// Ist ein echter LLM-Aufruf konfiguriert?
    pub fn is_configured(&self) -> bool {
        self.model
            .as_deref()
            .map(str::trim)
            .is_some_and(|m| !m.is_empty())
    }
}

impl fmt::Debug for LlmConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LlmConfig")
            .field("endpoint", &self.endpoint)
            .field("key", &mask_secret(self.key.as_deref()))
            .field("model", &self.model)
            .field("timeout_s", &self.timeout_s)
            .field("temperature", &self.temperature)
            .field("max_tokens", &self.max_tokens)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    pub level: String,
    /// "text" (Development) | "json" (Service/Machine).
    pub format: String,
    /// Anzahl aufzubewahrender Roll-Dateien.
    pub retention: usize,
    /// Opt-in: komplette Prompts loggen (Spec §17). Default: aus.
    pub prompt_log: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: "text".to_string(),
            retention: 7,
            prompt_log: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VerifyConfig {
    /// Unterer Schwellwert für semantic_preservation (0..1).
    pub threshold: f64,
    /// Maximale Re-Optimize-Versuche (Endlosschleifen-Schutz).
    pub max_attempts: usize,
}

impl Default for VerifyConfig {
    fn default() -> Self {
        Self {
            threshold: 0.85,
            max_attempts: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8770,
        }
    }
}

/// Datei-Konfiguration (`config/config.toml`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FileConfig {
    pub provider: Option<String>,
    pub llm: LlmConfig,
    pub log: LogConfig,
    pub verify: VerifyConfig,
    pub service: ServiceConfig,
    #[serde(default)]
    pub python_paths: Vec<String>,
}

/// Voll aufgelöste Anwendungskonfiguration.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub home: PathBuf,
    pub provider: ProviderKind,
    pub llm: LlmConfig,
    pub log: LogConfig,
    pub verify: VerifyConfig,
    pub service: ServiceConfig,
    /// Zusätzliche Python-Importpfade (PF_PYTHON_PATH, ':'-separiert).
    pub python_paths: Vec<PathBuf>,
}

impl AppConfig {
    /// Lädt die Konfiguration; `home_override` ersetzt die Auflösung
    /// (PF_HOME > $HOME/.prompt-forge) — für Tests.
    pub fn load(home_override: Option<&Path>) -> Result<Self> {
        let home = match home_override {
            Some(p) => p.to_path_buf(),
            None => crate::path::resolve_home()?,
        };

        let mut file = FileConfig::default();
        let config_file = home.join("config").join("config.toml");
        if config_file.is_file() {
            let raw = std::fs::read_to_string(&config_file).map_err(|e| {
                err(
                    ErrorKind::Configuration,
                    format!("{} lesen: {e}", config_file.display()),
                )
            })?;
            file = toml::from_str(&raw).map_err(|e| {
                err(
                    ErrorKind::Configuration,
                    format!("{} parsen: {e}", config_file.display()),
                )
            })?;
        }

        let provider = env_provider()
            .and_then(|s| ProviderKind::parse(&s))
            .or_else(|| file.provider.as_deref().and_then(ProviderKind::parse))
            .unwrap_or_default();

        let llm = LlmConfig {
            endpoint: env_opt("LLM_ENDPOINT").or(file.llm.endpoint),
            key: env_opt("LLM_KEY").or(file.llm.key),
            model: env_opt("LLM_MODEL").or(file.llm.model),
            timeout_s: env_u64("LLM_TIMEOUT_S").unwrap_or(file.llm.timeout_s),
            temperature: env_f64("LLM_TEMPERATURE").or(file.llm.temperature),
            max_tokens: env_u32("LLM_MAX_TOKENS").or(file.llm.max_tokens),
        };

        let log = LogConfig {
            level: env_opt("PF_LOG_LEVEL").unwrap_or(file.log.level),
            format: env_opt("PF_LOG_FORMAT").unwrap_or(file.log.format),
            retention: env_usize("PF_LOG_RETENTION").unwrap_or(file.log.retention),
            prompt_log: env_bool("PF_PROMPT_LOG").unwrap_or(file.log.prompt_log),
        };

        let verify = VerifyConfig {
            threshold: env_f64("PF_VERIFY_THRESHOLD").unwrap_or(file.verify.threshold),
            max_attempts: env_usize("PF_MAX_ATTEMPTS").unwrap_or(file.verify.max_attempts),
        };

        let service = ServiceConfig {
            host: env_opt("PF_SERVICE_HOST").unwrap_or(file.service.host),
            port: env_u16("PF_SERVICE_PORT").unwrap_or(file.service.port),
        };

        let mut python_paths: Vec<PathBuf> = file.python_paths.iter().map(PathBuf::from).collect();
        if let Some(p) = env_opt("PF_PYTHON_PATH") {
            for part in p.split(':') {
                if !part.is_empty() {
                    python_paths.push(PathBuf::from(part));
                }
            }
        }

        Ok(Self {
            home,
            provider,
            llm,
            log,
            verify,
            service,
            python_paths,
        })
    }

    /// Effektive Provider-Entscheidung (Auto-Auflösung).
    pub fn effective_provider(&self) -> ProviderKind {
        match self.provider {
            ProviderKind::Auto => {
                if self.llm.is_configured() {
                    ProviderKind::AnyLlm
                } else {
                    ProviderKind::None
                }
            }
            other => other,
        }
    }

    pub fn config_file_path(&self) -> PathBuf {
        self.home.join("config").join("config.toml")
    }

    /// Secret-freie Zusammenfassung für Statusausgaben.
    pub fn summary(&self) -> String {
        format!(
            "home={} provider={:?} llm(endpoint={:?}, model={:?}, key={}) log(level={}, format={})",
            self.home.display(),
            self.effective_provider(),
            self.llm.endpoint,
            self.llm.model,
            mask_secret(self.llm.key.as_deref()),
            self.log.level,
            self.log.format
        )
    }
}

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}
fn env_u64(name: &str) -> Option<u64> {
    env_opt(name).and_then(|v| v.parse().ok())
}
fn env_u32(name: &str) -> Option<u32> {
    env_opt(name).and_then(|v| v.parse().ok())
}
fn env_u16(name: &str) -> Option<u16> {
    env_opt(name).and_then(|v| v.parse().ok())
}
fn env_usize(name: &str) -> Option<usize> {
    env_opt(name).and_then(|v| v.parse().ok())
}
fn env_f64(name: &str) -> Option<f64> {
    env_opt(name).and_then(|v| v.parse().ok())
}
fn env_bool(name: &str) -> Option<bool> {
    env_opt(name).and_then(|v| match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    })
}

fn env_provider() -> Option<String> {
    env_opt("PF_PROVIDER")
}

/// Default-Config-Inhalt für `prompt-forge init`.
pub const DEFAULT_CONFIG_TOML: &str = r#"# PromptForge-Konfiguration (~/.prompt-forge/config/config.toml)
# Reihenfolge: Defaults < diese Datei < Environment-Variablen.

# provider: auto | any_llm | mock | none  (auto = LLM, sobald Modell gesetzt)
# provider = "auto"

[llm]
# Beliebiges OpenAI-kompatibles Ende (lokal oder Cloud):
# endpoint = "http://localhost:11434/v1"   # z. B. Ollama
# key = ""                                  # leer für lokale Server
# model = "qwen2.5-coder:7b"
timeout_s = 120
# temperature = 0.2
# max_tokens = 4096

[log]
level = "info"        # debug | info | warning | error
format = "text"       # text | json
retention = 7         # Rolling-Dateien (prompt-forge.log, .1.log, …)
prompt_log = false    # Opt-in: komplette Prompts loggen (Vorsicht: sensibel!)

[verify]
threshold = 0.85      # Mindest-Semantik-Erhalt (0..1)
max_attempts = 2      # Re-Optimize-Versuche (Endlosschleifen-Schutz)

[service]
host = "127.0.0.1"
port = 8770
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pf-cfg-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("config")).unwrap();
        dir
    }

    #[test]
    fn default_config_loads() {
        let dir = temp_home();
        let cfg = AppConfig::load(Some(&dir)).unwrap();
        assert_eq!(cfg.home, dir);
        assert_eq!(cfg.effective_provider(), ProviderKind::None);
        assert_eq!(cfg.log.format, "text");
        assert_eq!(cfg.service.port, 8770);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn file_config_is_read() {
        let dir = temp_home();
        let toml = r#"
[llm]
endpoint = "http://localhost:1234/v1"
model = "local-model"

[verify]
threshold = 0.9
max_attempts = 3
"#;
        std::fs::write(dir.join("config/config.toml"), toml).unwrap();
        let cfg = AppConfig::load(Some(&dir)).unwrap();
        assert_eq!(
            cfg.llm.endpoint.as_deref(),
            Some("http://localhost:1234/v1")
        );
        assert_eq!(cfg.llm.model.as_deref(), Some("local-model"));
        assert_eq!(cfg.verify.threshold, 0.9);
        assert_eq!(cfg.verify.max_attempts, 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn env_overrides_file() {
        let dir = temp_home();
        std::fs::write(
            dir.join("config/config.toml"),
            "[llm]\nmodel = \"file-model\"\n",
        )
        .unwrap();
        // SAFETY: Test isoliert; kein paralleler Zugriff auf LLM_MODEL.
        unsafe { std::env::set_var("LLM_MODEL", "env-model") };
        let cfg = AppConfig::load(Some(&dir)).unwrap();
        assert_eq!(cfg.llm.model.as_deref(), Some("env-model"));
        // SAFETY: Test isoliert.
        unsafe { std::env::remove_var("LLM_MODEL") };
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn debug_output_never_shows_secret() {
        let cfg = AppConfig {
            home: PathBuf::from("/tmp/x"),
            provider: ProviderKind::AnyLlm,
            llm: LlmConfig {
                key: Some("sk-supergeheim-123456".to_string()),
                model: Some("m".to_string()),
                ..Default::default()
            },
            log: LogConfig::default(),
            verify: VerifyConfig::default(),
            service: ServiceConfig::default(),
            python_paths: vec![],
        };
        let s = format!("{:?}", cfg);
        assert!(!s.contains("sk-supergeheim"));
        assert!(s.contains("REDACTED"));
    }

    #[test]
    fn provider_parse() {
        assert_eq!(ProviderKind::parse("mock"), Some(ProviderKind::Mock));
        assert_eq!(ProviderKind::parse("any_llm"), Some(ProviderKind::AnyLlm));
        assert_eq!(ProviderKind::parse("bogus"), None);
    }

    #[test]
    fn default_config_toml_is_valid_toml() {
        let parsed: FileConfig = toml::from_str(DEFAULT_CONFIG_TOML).unwrap();
        assert_eq!(parsed.verify.max_attempts, 2);
    }
}
