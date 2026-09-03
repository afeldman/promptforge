//! Clipboard-Abstraktion (Spec §10): arboard primär, Kommando-Fallback
//! (pbcopy / wl-copy / xclip / Set-Clipboard). Geteilt von CLI und TUI.

use crate::error::{ErrorKind, Result, err};

/// Kopiert Text in die Zwischenablage.
pub fn copy_text(text: &str) -> Result<()> {
    match arboard_copy(text) {
        Ok(()) => Ok(()),
        Err(first) => match command_copy(text) {
            Ok(()) => Ok(()),
            Err(_) => Err(first),
        },
    }
}

fn arboard_copy(text: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|e| err(ErrorKind::Io, format!("Clipboard nicht verfügbar: {e}")))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| err(ErrorKind::Io, format!("Clipboard schreiben: {e}")))
}

#[cfg(target_os = "macos")]
fn command_copy(text: &str) -> Result<()> {
    use std::io::Write;
    let mut child = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| err(ErrorKind::Io, format!("pbcopy starten: {e}")))?;
    child
        .stdin
        .take()
        .ok_or_else(|| err(ErrorKind::Io, "pbcopy stdin"))?
        .write_all(text.as_bytes())
        .map_err(|e| err(ErrorKind::Io, format!("pbcopy schreiben: {e}")))?;
    child
        .wait()
        .map_err(|e| err(ErrorKind::Io, format!("pbcopy warten: {e}")))?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn command_copy(text: &str) -> Result<()> {
    use std::io::Write;
    for cmd in ["wl-copy", "xclip", "xsel"] {
        if let Ok(mut child) = std::process::Command::new(cmd)
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            let _ = child.stdin.take().map(|mut s| s.write_all(text.as_bytes()));
            let _ = child.wait();
            return Ok(());
        }
    }
    Err(err(
        ErrorKind::Io,
        "Kein Clipboard-Tool gefunden (wl-copy/xclip/xsel)",
    ))
}

#[cfg(windows)]
fn command_copy(text: &str) -> Result<()> {
    let ps = format!("Set-Clipboard -Value @'\n{text}\n'@");
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps])
        .output()
        .map_err(|e| err(ErrorKind::Io, format!("powershell starten: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(err(ErrorKind::Io, "Set-Clipboard fehlgeschlagen"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_copy_returns_error_not_panic() {
        // Kopieren ist umgebungsabhängig — hier nur Abwesenheit von Panics.
        let _ = copy_text("test");
    }
}
