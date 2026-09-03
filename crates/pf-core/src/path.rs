//! Pfad-/Layout-Logik für das Runtime-User-Home (Spec §14).

use std::path::{Path, PathBuf};

use crate::error::{ErrorKind, Result, err};

/// Struktur des User-Home-Verzeichnisses.
#[derive(Debug, Clone)]
pub struct HomeLayout {
    pub root: PathBuf,
    pub config_dir: PathBuf,
    pub prompt_dir: PathBuf,
    pub templates_dir: PathBuf,
    pub generated_dir: PathBuf,
    pub optimized_dir: PathBuf,
    pub archive_dir: PathBuf,
    pub history_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub state_dir: PathBuf,
}

impl HomeLayout {
    pub fn from_root(root: &Path) -> Self {
        let prompt_dir = root.join("prompt");
        Self {
            root: root.to_path_buf(),
            config_dir: root.join("config"),
            templates_dir: prompt_dir.join("templates"),
            generated_dir: prompt_dir.join("generated"),
            optimized_dir: prompt_dir.join("optimized"),
            archive_dir: prompt_dir.join("archive"),
            prompt_dir,
            history_dir: root.join("history"),
            cache_dir: root.join("cache"),
            logs_dir: root.join("logs"),
            state_dir: root.join("state"),
        }
    }

    /// Legt alle benötigten Verzeichnisse an (idempotent).
    pub fn ensure(&self) -> Result<()> {
        for dir in [
            &self.config_dir,
            &self.prompt_dir,
            &self.templates_dir,
            &self.generated_dir,
            &self.optimized_dir,
            &self.archive_dir,
            &self.history_dir,
            &self.cache_dir,
            &self.logs_dir,
            &self.state_dir,
        ] {
            std::fs::create_dir_all(dir).map_err(|e| {
                err(
                    ErrorKind::Persistence,
                    format!("Verzeichnis {} anlegen: {e}", dir.display()),
                )
            })?;
        }
        Ok(())
    }
}

/// Auflösung des User-Home: `PF_HOME` > `$HOME/.prompt-forge`.
pub fn resolve_home() -> Result<PathBuf> {
    if let Some(h) = std::env::var_os("PF_HOME") {
        let p = PathBuf::from(h);
        if !p.is_absolute() {
            return Err(err(ErrorKind::Configuration, "PF_HOME muss absolut sein"));
        }
        return Ok(p);
    }
    match std::env::var_os("HOME") {
        Some(h) => Ok(PathBuf::from(h).join(".prompt-forge")),
        None => Err(err(
            ErrorKind::Configuration,
            "Weder PF_HOME noch HOME gesetzt — kann User-Home nicht bestimmen",
        )),
    }
}

pub fn request_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Zeitstempel für Dateinamen: `20260903T153045123` (lokal).
pub fn file_timestamp() -> String {
    chrono::Local::now().format("%Y%m%dT%H%M%S%3f").to_string()
}

/// RFC3339-Zeitstempel für Metadaten/History.
pub fn rfc3339_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_has_all_spec_dirs() {
        let l = HomeLayout::from_root(Path::new("/x/.prompt-forge"));
        assert_eq!(l.config_dir, Path::new("/x/.prompt-forge/config"));
        assert_eq!(
            l.templates_dir,
            Path::new("/x/.prompt-forge/prompt/templates")
        );
        assert_eq!(
            l.generated_dir,
            Path::new("/x/.prompt-forge/prompt/generated")
        );
        assert_eq!(
            l.optimized_dir,
            Path::new("/x/.prompt-forge/prompt/optimized")
        );
        assert_eq!(l.archive_dir, Path::new("/x/.prompt-forge/prompt/archive"));
        assert_eq!(l.history_dir, Path::new("/x/.prompt-forge/history"));
        assert_eq!(l.logs_dir, Path::new("/x/.prompt-forge/logs"));
    }

    #[test]
    fn ensure_creates_dirs_in_temp() {
        let tmp = std::env::temp_dir().join(format!("pf-test-{}", uuid::Uuid::new_v4()));
        let l = HomeLayout::from_root(&tmp);
        l.ensure().unwrap();
        assert!(l.optimized_dir.is_dir());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn request_ids_unique() {
        assert_ne!(request_id(), request_id());
    }

    #[test]
    fn timestamps_are_lexicographically_sortable() {
        let a = file_timestamp();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = file_timestamp();
        assert!(b > a);
    }
}
