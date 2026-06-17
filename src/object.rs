//! The two things Lore stores, both content addressed by SHA-256.
//!
//! An [`Entry`] is one unit of intent: a prompt, a note, or a decision. Its id
//! is the hash of its `kind` and `text` only, so the same intent expressed
//! twice collapses to a single object. That is what makes merges deduplicate
//! for free. A [`Commit`] groups entries and points at its parents; its id
//! covers every field, because a commit is a unique event in history.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One unit of intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Free-form category, conventionally `prompt`, `note`, or `decision`.
    pub kind: String,
    /// Who first recorded this intent.
    pub author: String,
    /// Unix nanoseconds when it was first recorded.
    pub timestamp: u64,
    /// The intent itself.
    pub text: String,
}

/// A group of entries with links to its parent commits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    /// Parent commit ids. Empty for the root, two or more for a merge.
    pub parents: Vec<String>,
    pub author: String,
    pub timestamp: u64,
    pub message: String,
    /// Entry ids recorded by this commit.
    pub entries: Vec<String>,
}

/// A stored object, tagged so it round-trips through JSON unambiguously.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Object {
    Entry(Entry),
    Commit(Commit),
}

impl Object {
    /// The content address of this object as lowercase hex.
    pub fn id(&self) -> String {
        let mut h = Sha256::new();
        match self {
            // Identity is intent only, so duplicate intent deduplicates.
            Object::Entry(e) => {
                h.update(b"entry\0");
                h.update(e.kind.as_bytes());
                h.update(b"\0");
                h.update(e.text.as_bytes());
            }
            // A commit is a unique event, so identity covers everything.
            Object::Commit(c) => {
                h.update(b"commit\0");
                h.update(serde_json::to_vec(c).expect("commit serializes"));
            }
        }
        hex(&h.finalize())
    }

    /// The serialized bytes written to the object store.
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("object serializes")
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &str, author: &str, ts: u64, text: &str) -> Object {
        Object::Entry(Entry {
            kind: kind.into(),
            author: author.into(),
            timestamp: ts,
            text: text.into(),
        })
    }

    #[test]
    fn entry_id_is_intent_only() {
        // Same kind and text but different author and time share one id.
        let a = entry("note", "alice", 1, "use sqlite");
        let b = entry("note", "bob", 999, "use sqlite");
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn entry_id_changes_with_kind_or_text() {
        let base = entry("note", "alice", 1, "use sqlite");
        assert_ne!(base.id(), entry("decision", "alice", 1, "use sqlite").id());
        assert_ne!(base.id(), entry("note", "alice", 1, "use postgres").id());
    }

    #[test]
    fn commit_id_covers_every_field() {
        let mk = |msg: &str, ts: u64| {
            Object::Commit(Commit {
                parents: vec![],
                author: "alice".into(),
                timestamp: ts,
                message: msg.into(),
                entries: vec!["e1".into()],
            })
        };
        assert_eq!(mk("init", 1).id(), mk("init", 1).id());
        assert_ne!(mk("init", 1).id(), mk("init", 2).id());
        assert_ne!(mk("init", 1).id(), mk("start", 1).id());
    }

    #[test]
    fn id_is_lowercase_hex_of_expected_length() {
        let id = entry("note", "alice", 1, "x").id();
        assert_eq!(id.len(), 64);
        assert!(id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    }

    #[test]
    fn bytes_round_trip_through_json() {
        let obj = entry("prompt", "alice", 7, "make it fast");
        let back: Object = serde_json::from_slice(&obj.to_bytes()).unwrap();
        assert_eq!(obj, back);
    }
}
