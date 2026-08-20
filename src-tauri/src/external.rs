//! Following a link the document points at, out to the user's browser.
//!
//! Nothing here trusts the PDF. A document can carry any URI string it likes,
//! so the scheme allowlist below is the boundary between "somewhere the user
//! asked to go" and "an instruction to run something local". The frontend
//! asks the user before this is ever reached; this is the gate the document
//! cannot talk its way past.

use crate::errors::{AppError, AppResult};

/// Schemes a document may send the user to. Everything else is refused —
/// `file:` reaches the local disk, `javascript:` and `data:` execute, and a
/// custom scheme hands control to whatever program claimed it.
const ALLOWED_SCHEMES: &[&str] = &["http", "https", "mailto"];

/// Longest link we will look at. Real links are nowhere near this; the cap
/// just keeps a pathological annotation from reaching the platform opener.
const MAX_URL_BYTES: usize = 4_096;

/// Parse and normalize a document link, or refuse it.
///
/// The *reserialized* URL is what gets returned, not the document's original
/// bytes: parsing percent-encodes whatever was embedded, so quoting and
/// argument tricks cannot survive as literal characters into the command the
/// platform opener builds.
pub fn sanitize_external_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_URL_BYTES {
        return None;
    }
    let parsed = url::Url::parse(trimmed).ok()?;
    if !ALLOWED_SCHEMES.contains(&parsed.scheme()) {
        return None;
    }
    // A hostless http(s) URL has nowhere to go and tends to mean the parser
    // recovered something odd out of a malformed annotation.
    if parsed.scheme() != "mailto" && parsed.host().is_none() {
        return None;
    }
    let normalized = parsed.to_string();
    // A control character surviving normalization would mean the parser and
    // whatever consumes the string next disagree about where the URL ends.
    if normalized.chars().any(char::is_control) {
        return None;
    }
    Some(normalized)
}

/// Hand a validated link to the platform's default browser.
pub fn open_external_url(raw: &str) -> AppResult<()> {
    let url = sanitize_external_url(raw)
        .ok_or_else(|| AppError::Unsupported(format!("refusing to open this link: {raw}")))?;
    spawn_opener(&url)
}

#[cfg(target_os = "macos")]
fn spawn_opener(url: &str) -> AppResult<()> {
    spawn(std::process::Command::new("open").arg(url))
}

#[cfg(target_os = "linux")]
fn spawn_opener(url: &str) -> AppResult<()> {
    spawn(std::process::Command::new("xdg-open").arg(url))
}

#[cfg(target_os = "windows")]
fn spawn_opener(url: &str) -> AppResult<()> {
    // Deliberately not `cmd /C start`, which re-parses its arguments through
    // the shell. rundll32 takes the URL as one argument and passes it to the
    // registered protocol handler untouched.
    spawn(std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]))
}

/// Detached: the browser outlives us, and we neither wait for it nor read it.
fn spawn(command: &mut std::process::Command) -> AppResult<()> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| AppError::Io(format!("could not open link: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_web_and_mail_links() {
        assert_eq!(
            sanitize_external_url("https://arxiv.org/abs/1706.03762").as_deref(),
            Some("https://arxiv.org/abs/1706.03762")
        );
        assert_eq!(
            sanitize_external_url("http://example.com/a?b=c#d").as_deref(),
            Some("http://example.com/a?b=c#d")
        );
        assert!(sanitize_external_url("mailto:someone@example.edu").is_some());
    }

    #[test]
    fn refuses_schemes_that_reach_the_local_machine() {
        // The whole point of the allowlist: a PDF gets to name a destination,
        // not to name a program.
        for hostile in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "vscode://file/Users/me/.ssh/id_rsa",
            "smb://198.51.100.4/share",
        ] {
            assert!(
                sanitize_external_url(hostile).is_none(),
                "should have refused {hostile}"
            );
        }
    }

    #[test]
    fn refuses_malformed_hostless_and_oversized_links() {
        assert!(sanitize_external_url("").is_none());
        assert!(sanitize_external_url("   ").is_none());
        assert!(sanitize_external_url("not a url at all").is_none());
        assert!(sanitize_external_url("https://").is_none());
        let huge = format!("https://example.com/{}", "a".repeat(MAX_URL_BYTES));
        assert!(sanitize_external_url(&huge).is_none());
    }

    #[test]
    fn normalization_encodes_what_the_document_embedded() {
        // Whitespace and quotes in an annotation must not survive as literal
        // characters into whatever builds the opener's command line.
        let sanitized = sanitize_external_url("https://example.com/a b\"c").expect("valid");
        assert!(!sanitized.contains(' '), "{sanitized}");
        assert!(!sanitized.contains('"'), "{sanitized}");
    }

    #[test]
    fn output_never_carries_control_characters() {
        // The parser neutralizes these rather than rejecting them — a NUL is
        // percent-encoded, and tabs/newlines are stripped outright. What
        // matters is the invariant on the way out, since that string is what
        // the platform opener consumes. The guard in the sanitizer is a
        // backstop for the day a parser update stops doing this.
        for raw in [
            "https://example.com/\u{0}x",
            "https://example.com/a\nb",
            "https://example.com/a\tb",
        ] {
            let sanitized = sanitize_external_url(raw).expect("host is valid");
            assert!(
                !sanitized.chars().any(char::is_control),
                "{raw:?} normalized to {sanitized:?}"
            );
        }
    }
}
