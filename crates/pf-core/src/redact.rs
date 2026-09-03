//! Redaction sensibler Werte (Spec §16/§17).
//!
//! Grundsatz: Secrets (API-Keys, Authorization-Werte, Tokens) dürfen niemals
//! in Logs erscheinen. Zusätzlich zur Substitution bekannter Secret-Werte
//! werden typische Secret-Formen erkannt und maskiert.

/// Ersetzt bekannte Secret-Werte und typische Secret-Formen in `line`.
pub fn sanitize_line(line: &str, secrets: &[String]) -> String {
    let mut out = line.to_string();
    for secret in secrets {
        if secret.len() >= 6 {
            out = out.replace(secret, "[REDACTED]");
        }
    }
    out = mask_bearer(&out);
    out = mask_sk_tokens(&out);
    mask_assignment_secrets(&out)
}

/// `Authorization: Bearer <token>` → `Authorization: Bearer [REDACTED]`
fn mask_bearer(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(idx) = rest.to_ascii_lowercase().find("bearer ") {
        out.push_str(&rest[..idx + "bearer ".len()]);
        let tail = &rest[idx + "bearer ".len()..];
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',')
            .unwrap_or(tail.len());
        let token = &tail[..end];
        out.push_str(&mask_token(token));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// `sk-…`-Tokens (OpenAI/Anthropic-Stil) maskieren.
fn mask_sk_tokens(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(idx) = rest.find("sk-") {
        out.push_str(&rest[..idx]);
        let tail = &rest[idx..];
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',')
            .unwrap_or(tail.len());
        let token = &tail[..end];
        out.push_str(&mask_token(token));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Werte von `key=…`-artigen Zuweisungen maskieren (z. B. `api_key=xyz`).
fn mask_assignment_secrets(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(idx) = find_key_assignment(rest) {
        out.push_str(&rest[..idx]);
        let tail = &rest[idx..];
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',')
            .unwrap_or(tail.len());
        let value = &tail[..end];
        let (prefix, _) = value.split_once('=').unwrap_or((value, ""));
        out.push_str(prefix);
        out.push('=');
        out.push_str(&mask_token(&value[prefix.len() + 1..]));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Findet `key=`-Zuweisungen, deren Schlüssel sensitiv klingt.
fn find_key_assignment(s: &str) -> Option<usize> {
    const KEYS: &[&str] = &[
        "api_key=",
        "apikey=",
        "api-key=",
        "secret=",
        "password=",
        "token=",
        "key=",
        "authorization=",
    ];
    let lower = s.to_ascii_lowercase();
    KEYS.iter().filter_map(|k| lower.find(k)).min()
}

/// Maskiert ein Token: kurze Werte vollständig, längere mit sichtbarem Ende.
fn mask_token(token: &str) -> String {
    if token.is_empty() {
        return String::new();
    }
    if token.len() <= 8 {
        return "[REDACTED]".to_string();
    }
    let visible = token.len() - 4;
    format!("[REDACTED:…{}]", &token[visible..])
}

/// Kurzform für Debug-Ausgaben: nur letzte 4 Zeichen eines Secrets.
pub fn mask_secret(secret: Option<&str>) -> String {
    match secret {
        None => "none".to_string(),
        Some(s) if s.len() <= 8 => "[REDACTED]".to_string(),
        Some(s) => {
            let keep = s.len() - 4;
            format!("[REDACTED:…{}]", &s[keep..])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secrets(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn replaces_known_secret_value() {
        let out = sanitize_line(
            "key is sk-abcdefghijkl1234 end",
            &secrets(&["sk-abcdefghijkl1234"]),
        );
        assert!(!out.contains("sk-abcdefghijkl1234"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn masks_bearer_tokens() {
        let out = sanitize_line("Authorization: Bearer 0123456789abcdef", &[]);
        assert!(!out.contains("0123456789abcdef"));
        assert!(out.contains("Bearer [REDACTED"));
    }

    #[test]
    fn masks_sk_tokens_generic() {
        let out = sanitize_line("token=sk-proj-AAAA-BBBB-CCCC-DDDD", &[]);
        assert!(!out.contains("AAAA-BBBB"));
        assert!(out.contains("REDACTED"));
    }

    #[test]
    fn masks_key_assignment() {
        let out = sanitize_line("api_key=supersecretvalue123", &[]);
        assert!(!out.contains("supersecretvalue123"));
        assert!(out.contains("api_key=[REDACTED"));
    }

    #[test]
    fn leaves_plain_text_untouched() {
        let line = "compile stage finished request_id=abc123";
        assert_eq!(sanitize_line(line, &[]), line);
    }

    #[test]
    fn mask_secret_short_and_long() {
        assert_eq!(mask_secret(None), "none");
        assert_eq!(mask_secret(Some("kurz")), "[REDACTED]");
        assert_eq!(mask_secret(Some("0123456789abcdef")), "[REDACTED:…cdef]");
        assert!(!mask_secret(Some("0123456789abcdef")).contains("0123456789ab"));
    }
}
