//! @worker mention parsing (RR-0045, Invariant 17).
//!
//! Mentions are STRUCTURAL addressing: `@backend fix the build` routes to
//! the worker whose display name or alias is "backend". Resolution happens
//! against a directory the caller supplies (current names + aliases), so a
//! rename never breaks addressing — the old name resolves as an alias
//! (Invariant 43).

use crate::ids::WorkerId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mention {
    /// The literal text after `@`, as typed.
    pub raw: String,
    /// Byte offset of the `@` in the source text.
    pub offset: usize,
}

/// A directory entry for resolution: one worker, its current display name,
/// and every name it has ever had (aliases accumulate on rename).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub worker: WorkerId,
    pub display_name: String,
    pub aliases: Vec<String>,
    /// Group names resolve too — a group mention fans out (Invariant 29).
    pub is_group: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Resolution {
    Worker { id: WorkerId },
    Group { id: WorkerId },
    /// No entry matched — surfaced, never silently dropped (a mention that
    /// vanishes is a message that was never delivered and nobody knows).
    Unresolved { raw: String },
}

/// Extract mentions: `@` followed by [A-Za-z0-9_-]+, not preceded by a word
/// character (so `a@b.com` is an email, not a mention).
pub fn parse_mentions(text: &str) -> Vec<Mention> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let preceded_by_word = i > 0
                && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            if !preceded_by_word {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric()
                        || bytes[end] == b'-'
                        || bytes[end] == b'_')
                {
                    end += 1;
                }
                if end > start {
                    out.push(Mention {
                        raw: text[start..end].to_string(),
                        offset: i,
                    });
                    i = end;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Resolve a mention against the directory. Match order: exact display name,
/// then alias — both case-insensitive. Ambiguity (two workers claiming the
/// same alias) resolves to the CURRENT display-name holder first; among
/// aliases, the first directory entry wins deterministically.
pub fn resolve(mention: &Mention, directory: &[DirectoryEntry]) -> Resolution {
    let needle = mention.raw.to_lowercase();
    if let Some(e) = directory
        .iter()
        .find(|e| e.display_name.to_lowercase() == needle)
    {
        return if e.is_group {
            Resolution::Group {
                id: e.worker.clone(),
            }
        } else {
            Resolution::Worker {
                id: e.worker.clone(),
            }
        };
    }
    if let Some(e) = directory
        .iter()
        .find(|e| e.aliases.iter().any(|a| a.to_lowercase() == needle))
    {
        return if e.is_group {
            Resolution::Group {
                id: e.worker.clone(),
            }
        } else {
            Resolution::Worker {
                id: e.worker.clone(),
            }
        };
    }
    Resolution::Unresolved {
        raw: mention.raw.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wid(n: u128) -> WorkerId {
        WorkerId::from_ulid(ulid::Ulid::from_parts(1_700_000_000_000, n))
    }

    fn dir() -> Vec<DirectoryEntry> {
        vec![
            DirectoryEntry {
                worker: wid(1),
                display_name: "backend".into(),
                aliases: vec!["api-server".into()],
                is_group: false,
            },
            DirectoryEntry {
                worker: wid(2),
                display_name: "social-media".into(),
                aliases: vec![],
                is_group: false,
            },
            DirectoryEntry {
                worker: wid(3),
                display_name: "gtm".into(),
                aliases: vec![],
                is_group: true,
            },
        ]
    }

    #[test]
    fn parses_mentions_and_skips_emails() {
        let ms = parse_mentions("hey @backend and @social-media, mail ethan@amux.io; also @gtm");
        let names: Vec<&str> = ms.iter().map(|m| m.raw.as_str()).collect();
        assert_eq!(names, vec!["backend", "social-media", "gtm"]);
    }

    #[test]
    fn mention_at_start_and_bare_at_ignored(){
        assert_eq!(parse_mentions("@backend go").len(), 1);
        assert_eq!(parse_mentions("nothing @ all").len(), 0);
        assert_eq!(parse_mentions("").len(), 0);
    }

    #[test]
    fn resolves_display_name_alias_and_group() {
        let d = dir();
        let m = |s: &str| Mention { raw: s.into(), offset: 0 };
        assert_eq!(resolve(&m("backend"), &d), Resolution::Worker { id: wid(1) });
        // Alias (old name after rename) still routes — Invariant 43.
        assert_eq!(resolve(&m("api-server"), &d), Resolution::Worker { id: wid(1) });
        // Case-insensitive.
        assert_eq!(resolve(&m("Backend"), &d), Resolution::Worker { id: wid(1) });
        assert_eq!(resolve(&m("gtm"), &d), Resolution::Group { id: wid(3) });
        assert_eq!(
            resolve(&m("nope"), &d),
            Resolution::Unresolved { raw: "nope".into() }
        );
    }

    #[test]
    fn current_name_outranks_someone_elses_alias() {
        // Worker 1 renamed FROM "scout"; worker 2 is now NAMED "scout".
        let d = vec![
            DirectoryEntry {
                worker: wid(1),
                display_name: "explorer".into(),
                aliases: vec!["scout".into()],
                is_group: false,
            },
            DirectoryEntry {
                worker: wid(2),
                display_name: "scout".into(),
                aliases: vec![],
                is_group: false,
            },
        ];
        let m = Mention { raw: "scout".into(), offset: 0 };
        // The CURRENT holder of the name wins over the historical alias.
        assert_eq!(resolve(&m, &d), Resolution::Worker { id: wid(2) });
    }
}
