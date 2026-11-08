//! Removes anything that could identify a person or a computer from text before it is shown in a
//! problem report. It runs on this machine, before the user sees the preview, so what they review
//! is exactly what would be sent.

use std::sync::LazyLock;

use regex::Regex;

/// Longest log excerpt a report may carry. The server enforces the same limit.
pub(crate) const MAX_LOG_CHARS: usize = 60_000;

struct Rule {
    pattern: Regex,
    replacement: &'static str,
}

fn rule(pattern: &str, replacement: &'static str) -> Rule {
    Rule {
        pattern: Regex::new(pattern).expect("anonymizer pattern is valid"),
        replacement,
    }
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        // File locations carry the Windows account name and often folder names the user chose.
        rule(r#"(?i)(?:\\\\\?\\)?[a-z]:[\\/][^\s"'<>|*?]*"#, "<path>"),
        rule(r#"\\\\[^\s"'<>|*?\\]+\\[^\s"'<>|*?]*"#, "<path>"),
        rule(
            r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b",
            "<id>",
        ),
        rule(r"\b7656119\d{10}\b", "<steamid>"),
        rule(r"(?i)\b[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}\b", "<email>"),
        // Addresses on the local machine are not identifying; every other IPv4 address is removed.
        rule(
            r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)\b",
            "<ip>",
        ),
        rule(r"(?i)\b(?:[0-9a-f]{1,4}:){3,7}[0-9a-f]{1,4}\b", "<ip>"),
        rule(
            r"(?i)\b(bearer|apikey|api_key|token|secret|password)(\s*[=:]\s*|\s+)\S+",
            "$1$2<secret>",
        ),
        rule(r"\b[A-Za-z0-9+/_-]{40,}={0,2}", "<secret>"),
        // A server's custom map title can name the server; the engine map name is kept.
        rule(
            r#"map=Some\("[^"]*[|\s][^"]*"\)"#,
            r#"map=Some("<custom>")"#,
        ),
    ]
});

/// Account and machine names are only known at run time, so they are removed by value.
#[derive(Debug, Clone, Default)]
pub(crate) struct Identity {
    terms: Vec<String>,
}

impl Identity {
    /// The names of the current Windows user and computer.
    pub(crate) fn current() -> Self {
        Self::from_terms(
            ["USERNAME", "COMPUTERNAME", "USERDOMAIN"]
                .into_iter()
                .filter_map(|name| std::env::var(name).ok()),
        )
    }

    pub(crate) fn from_terms(terms: impl IntoIterator<Item = String>) -> Self {
        // Very short names would also match ordinary words.
        let mut terms: Vec<String> = terms
            .into_iter()
            .map(|term| term.trim().to_owned())
            .filter(|term| term.chars().count() >= 3)
            .collect();
        terms.sort();
        terms.dedup();
        Self { terms }
    }
}

/// Anonymizes one piece of text (a log line, an error message or a note).
pub(crate) fn scrub(text: &str, identity: &Identity) -> String {
    let mut result = text.to_owned();
    for rule in RULES.iter() {
        result = rule
            .pattern
            .replace_all(&result, rule.replacement)
            .into_owned();
    }
    for term in &identity.terms {
        if let Ok(pattern) = Regex::new(&format!("(?i){}", regex::escape(term))) {
            result = pattern.replace_all(&result, "<user>").into_owned();
        }
    }
    result
}

/// A stable key for "the same problem", ignoring numbers and anything specific to one run.
pub(crate) fn fingerprint(kind: &str, message: &str) -> String {
    let normalized: String = message
        .to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphabetic())
        .filter(|word| !word.is_empty())
        .take(16)
        .collect::<Vec<_>>()
        .join(" ");
    // FNV-1a keeps the key short and the same across versions of the app.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in kind.bytes().chain(*b"|").chain(normalized.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> Identity {
        Identity::from_terms([
            "alexandra".to_owned(),
            "GAMING-PC".to_owned(),
            "ab".to_owned(),
        ])
    }

    #[test]
    fn file_paths_lose_account_and_folder_names() {
        let text = r"saved path=\\?\C:\Users\alexandra\Videos\2026-11-02\clip.mp4 and D:/Games/Steam/x.cfg";
        let cleaned = scrub(text, &identity());
        assert_eq!(cleaned, "saved path=<path> and <path>");
    }

    #[test]
    fn identifiers_addresses_and_secrets_are_removed() {
        let text = "match 9f22559a-9d08-48a2-9b09-9e02856b03e8 steam 76561198000000001 mail a.b@example.com \
                    from 192.168.1.20 and 127.0.0.1 token=abcdef123456";
        let cleaned = scrub(text, &identity());
        assert!(cleaned.contains("match <id>"), "{cleaned}");
        assert!(cleaned.contains("steam <steamid>"), "{cleaned}");
        assert!(cleaned.contains("mail <email>"), "{cleaned}");
        assert!(cleaned.contains("from <ip>"), "{cleaned}");
        assert!(cleaned.contains("token=<secret>"), "{cleaned}");
    }

    #[test]
    fn the_account_and_computer_names_are_removed_by_value() {
        let cleaned = scrub("hello Alexandra from gaming-pc, ab stays", &identity());
        assert_eq!(cleaned, "hello <user> from <user>, ab stays");
    }

    #[test]
    fn custom_server_map_titles_are_hidden_but_engine_maps_stay() {
        assert_eq!(
            scrub(
                r#"match started map=Some("DE_MIRAGE | RETAKE #61 | XPLAY.GG")"#,
                &identity()
            ),
            r#"match started map=Some("<custom>")"#
        );
        assert_eq!(
            scrub(r#"match started map=Some("de_mirage")"#, &identity()),
            r#"match started map=Some("de_mirage")"#
        );
    }

    #[test]
    fn ordinary_error_text_is_left_alone() {
        let text = "NVIDIA H.264 Encoder MFT returned no video after 180 pictures";
        assert_eq!(scrub(text, &identity()), text);
    }

    #[test]
    fn the_same_problem_has_the_same_fingerprint() {
        let one = fingerprint(
            "encoder_failed",
            "AMD H.264 Encoder returned no video after 180 pictures",
        );
        let two = fingerprint(
            "encoder_failed",
            "AMD H.264 Encoder returned no video after 90 pictures",
        );
        let other = fingerprint(
            "capture_error",
            "AMD H.264 Encoder returned no video after 90 pictures",
        );
        assert_eq!(one, two);
        assert_ne!(one, other);
    }
}
