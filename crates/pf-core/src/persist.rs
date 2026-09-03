//! Persistenz: atomare Datei-Writes, Artefakt-Namen, History (JSONL).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{ErrorKind, Result, err};

/// SHA-256-Hex eines Inhalts (für Dedupe/History, Spec: atomar/idempotent).
pub fn sha256_hex(content: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(content);
    hex_of(&h.finalize())
}

fn hex_of(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Atomarer Write (temp-Datei im Zielverzeichnis + rename).
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or_else(|| {
        err(
            ErrorKind::Persistence,
            format!("{} hat kein Elternverzeichnis", path.display()),
        )
    })?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        crate::path::request_id(),
        crate::path::file_timestamp()
    ));
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(content)?;
    f.flush()?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        err(
            ErrorKind::Persistence,
            format!("{} schreiben: {e}", path.display()),
        )
    })
}

/// Artefakt speichern: `<dir>/<prefix>-<timestamp>-<hash8>.<ext>`.
/// Liefert den endgültigen Pfad.
pub fn save_artifact(dir: &Path, prefix: &str, ext: &str, content: &str) -> Result<PathBuf> {
    let hash = &sha256_hex(content.as_bytes())[..8];
    let name = format!("{prefix}-{}-{hash}.{ext}", crate::path::file_timestamp());
    let path = dir.join(name);
    atomic_write(&path, content.as_bytes())?;
    Ok(path)
}

/// History-Eintrag (JSONL-Zeile, Spec §14/§17: Metadaten; Prompt-Inhalte
/// nur als Pfade bzw. opt-in).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HistoryEntry {
    pub request_id: String,
    pub created_at: String,
    #[serde(default)]
    pub intent: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub token_report: Option<serde_json::Value>,
    #[serde(default)]
    pub stages: Vec<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub ir_path: Option<String>,
    #[serde(default)]
    pub long_prompt_path: Option<String>,
    #[serde(default)]
    pub optimized_prompt_path: Option<String>,
    #[serde(default)]
    pub verification: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Hängt einen History-Eintrag als JSON-Zeile an (`history/history.jsonl`).
pub fn append_history(history_dir: &Path, entry: &HistoryEntry) -> Result<PathBuf> {
    std::fs::create_dir_all(history_dir)?;
    let file = history_dir.join("history.jsonl");
    let line = serde_json::to_string(entry)?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .map_err(|e| {
            err(
                ErrorKind::Persistence,
                format!("{} öffnen: {e}", file.display()),
            )
        })?;
    writeln!(f, "{line}")?;
    f.flush()?;
    Ok(file)
}

/// Liest die letzten `n` History-Einträge (neueste zuerst).
pub fn read_recent_history(history_dir: &Path, n: usize) -> Result<Vec<HistoryEntry>> {
    let file = history_dir.join("history.jsonl");
    if !file.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&file)?;
    let mut entries = Vec::new();
    for line in raw.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(e) = serde_json::from_str::<HistoryEntry>(line) {
            entries.push(e);
        }
        if entries.len() >= n {
            break;
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("pf-persist-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn sha256_is_hex_and_stable() {
        let a = sha256_hex(b"hello");
        let b = sha256_hex(b"hello");
        assert_eq!(a.len(), 64);
        assert_eq!(a, b);
        assert_ne!(a, sha256_hex(b"world"));
    }

    #[test]
    fn atomic_write_replaces_content() {
        let d = tmp_dir();
        let f = d.join("a.txt");
        atomic_write(&f, b"eins").unwrap();
        atomic_write(&f, b"zwei").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "zwei");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn save_artifact_names_unique_and_sorted() {
        let d = tmp_dir();
        let p1 = save_artifact(&d, "optimized", "md", "inhalt a").unwrap();
        let p2 = save_artifact(&d, "optimized", "md", "inhalt b").unwrap();
        assert_ne!(p1, p2);
        assert!(
            p1.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("optimized-")
        );
        assert!(p2.file_name().unwrap().to_string_lossy().ends_with(".md"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn history_append_and_read_back() {
        let d = tmp_dir();
        let e1 = HistoryEntry {
            request_id: "r1".to_string(),
            created_at: "t1".to_string(),
            status: "ok".to_string(),
            ..Default::default()
        };
        let e2 = HistoryEntry {
            request_id: "r2".to_string(),
            created_at: "t2".to_string(),
            status: "ok".to_string(),
            ..Default::default()
        };
        append_history(&d, &e1).unwrap();
        append_history(&d, &e2).unwrap();
        let recent = read_recent_history(&d, 1).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].request_id, "r2");
        let all = read_recent_history(&d, 10).unwrap();
        assert_eq!(all.len(), 2);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
