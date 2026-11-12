//! Version chains (#240 P3-3): order the files of one `VERSION_OF` group from oldest to newest using
//! only what the catalog already stores: the version tokens kept in the `VERSION_OF` edge evidence
//! (`v2`, `rev B`, `r3`, a date, `(2)`, `copy`) and, as the tiebreak, the file's modification time.
//! Pure functions over stored edges and `files` rows; no new tables, no new edge types.
//!
//! A group is ordered by token only when every member carries exactly one orderable token, all of the
//! same kind (`v` numbers, `rev` numbers, `rev` letters, dates, copy ordinals). Anything else (a mixed
//! set such as `v2` next to `rev B`, a member without a token, a status word alone) is ordered by
//! modification time, which is weaker: the chain says so. Where two neighbours are equal on both token
//! and mtime the order is by path and is labelled as such. Orders say what the names and times suggest,
//! never what the content proves.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog_edges::{is_status_word, FileRow, StoredEdge, MAX_VERSION_GROUP};

/// What put a file after its predecessor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// Version tokens compare as strictly older/newer (strong).
    Token,
    /// Modification time is strictly older (weak: copies and syncs rewrite mtimes).
    Mtime,
    /// Nothing distinguishes the two; path order, only so the result is deterministic (weakest).
    Path,
}

/// A group of versions, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    pub files: Vec<String>,
    /// Index of the asked-for path in `files`.
    pub position: usize,
    /// `bases[i]` is why `files[i + 1]` follows `files[i]`; one shorter than `files`.
    pub bases: Vec<Basis>,
}

impl Chain {
    /// True when any step rests on mtime or path rather than on tokens.
    pub fn is_weak(&self) -> bool {
        self.bases.iter().any(|b| *b != Basis::Token)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Version,
    RevNumber,
    RevLetter,
    Date,
    Copy,
}

type Key = (Kind, Vec<u64>);

enum Parsed {
    Orderable(Key),
    /// A status word: carries no order, ignored next to an orderable token.
    Neutral,
    /// Looks like a token but cannot be placed.
    Unknown,
}

fn digits(s: &str) -> Option<u64> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

fn letters(s: &str) -> Option<u64> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase()))
        .then(|| s.bytes().fold(0, |n, b| n * 26 + u64::from(b - b'a' + 1)))
}

fn date(s: &str) -> Option<Vec<u64>> {
    let parts: Vec<u64> = if s.len() == 8 {
        vec![digits(&s[..4])?, digits(&s[4..6])?, digits(&s[6..])?]
    } else {
        s.split('-').map(digits).collect::<Option<_>>()?
    };
    (parts.len() == 3 && (1..=12).contains(&parts[1]) && (1..=31).contains(&parts[2])).then_some(parts)
}

fn parse_token(t: &str) -> Parsed {
    let key = |kind, v: Vec<u64>| Parsed::Orderable((kind, v));
    if let Some(inner) = t.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        return digits(inner).map_or(Parsed::Unknown, |n| key(Kind::Copy, vec![n]));
    }
    if let Some(rest) = t.strip_prefix("copy") {
        let n = rest.rsplit(' ').next().and_then(digits).unwrap_or(1);
        return key(Kind::Copy, vec![n]);
    }
    if is_status_word(t) {
        return Parsed::Neutral;
    }
    if t.len() == 8 || t.len() == 10 {
        return date(t).map_or(Parsed::Unknown, |d| key(Kind::Date, d));
    }
    if let Some(rest) = t.strip_prefix("rev") {
        let rest = rest.trim_start_matches([' ', '-']);
        return match (digits(rest), letters(rest)) {
            (Some(n), _) => key(Kind::RevNumber, vec![n]),
            (_, Some(n)) => key(Kind::RevLetter, vec![n]),
            _ => Parsed::Unknown,
        };
    }
    if let Some(rest) = t.strip_prefix('r') {
        return digits(rest).map_or(Parsed::Unknown, |n| key(Kind::RevNumber, vec![n]));
    }
    if let Some(rest) = t.strip_prefix('v') {
        let nums: Option<Vec<u64>> = rest.split('.').map(digits).collect();
        return nums.map_or(Parsed::Unknown, |v| key(Kind::Version, v));
    }
    Parsed::Unknown
}

/// The one orderable key of a file's tokens; `None` if there is none or more than one, or any
/// token is unplaceable.
fn member_key(tokens: &[String]) -> Option<Key> {
    let mut found = None;
    for t in tokens {
        match parse_token(t) {
            Parsed::Orderable(k) if found.is_none() => found = Some(k),
            Parsed::Neutral => {}
            _ => return None,
        }
    }
    found
}

fn token_list(v: &serde_json::Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// Chain of the `VERSION_OF` group `path` belongs to. `edges` are `path`'s stored edges (as
/// `Catalog::edges_for` returns them; other kinds are ignored), `files` the catalog rows used for
/// mtime. `None` when `path` has no `VERSION_OF` edge or the group is larger than
/// `MAX_VERSION_GROUP` (no edges are stored for such groups, so a bigger one is not a document).
pub fn version_chain(path: &str, edges: &[StoredEdge], files: &[FileRow]) -> Option<Chain> {
    let mut tokens: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for e in edges.iter().filter(|e| e.kind == "VERSION_OF" && (e.src == path || e.dst == path)) {
        let pair = e.evidence.get("tokens").and_then(|t| t.as_array());
        for (i, end) in [e.src.as_str(), e.dst.as_str()].into_iter().enumerate() {
            tokens.entry(end).or_insert_with(|| pair.and_then(|p| p.get(i)).map(token_list).unwrap_or_default());
        }
    }
    if tokens.len() < 2 || tokens.len() > MAX_VERSION_GROUP {
        return None;
    }
    let mtime: BTreeMap<&str, i64> = files.iter().map(|f| (f.path.as_str(), f.modified)).collect();
    let keys: BTreeMap<&str, Option<Key>> = tokens.iter().map(|(p, t)| (*p, member_key(t))).collect();
    let kinds: BTreeSet<Option<Kind>> = keys.values().map(|k| k.as_ref().map(|k| k.0)).collect();
    // Tokens order the group only when every member has a key and all keys are one kind.
    let by_token = kinds.len() == 1 && !kinds.contains(&None);

    let mut order: Vec<&str> = tokens.keys().copied().collect();
    order.sort_by(|a, b| {
        let token = if by_token { keys[a].cmp(&keys[b]) } else { std::cmp::Ordering::Equal };
        token.then_with(|| mtime.get(a).cmp(&mtime.get(b))).then_with(|| a.cmp(b))
    });
    let bases = order
        .windows(2)
        .map(|w| {
            if by_token && keys[w[0]] < keys[w[1]] {
                Basis::Token
            } else if matches!((mtime.get(w[0]), mtime.get(w[1])), (Some(a), Some(b)) if a < b) {
                Basis::Mtime
            } else {
                Basis::Path
            }
        })
        .collect();
    let position = order.iter().position(|p| *p == path)?;
    Some(Chain { files: order.into_iter().map(str::to_string).collect(), position, bases })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_edges::{version_edges, EdgeType};

    fn row(path: &str, modified: i64) -> FileRow {
        FileRow { path: path.into(), size: 1, modified, content_hash: None }
    }

    /// Stored `VERSION_OF` edges touching `path`, built the way `rebuild_edges` builds them.
    fn edges_of(files: &[FileRow], path: &str) -> Vec<StoredEdge> {
        version_edges(files)
            .into_iter()
            .filter(|e| e.kind == EdgeType::VersionOf && (e.src == path || e.dst == path))
            .map(|e| StoredEdge {
                src: e.src,
                dst: e.dst,
                kind: "VERSION_OF".into(),
                confidence: e.confidence,
                evidence: e.evidence,
                class: "metadata".into(),
                created: 0,
                source_version: e.source_version,
            })
            .collect()
    }

    fn chain(files: &[FileRow], path: &str) -> Option<Chain> {
        version_chain(path, &edges_of(files, path), files)
    }

    fn names(c: &Chain) -> Vec<&str> {
        c.files.iter().map(|f| f.rsplit('/').next().unwrap()).collect()
    }

    #[test]
    fn version_numbers_order_numerically() {
        // Names and mtimes disagree with the numeric order on purpose.
        let f = [row("/d/spec v10.txt", 1), row("/d/spec v2.txt", 3), row("/d/spec v1.5.txt", 2)];
        let c = chain(&f, "/d/spec v2.txt").unwrap();
        assert_eq!(names(&c), ["spec v1.5.txt", "spec v2.txt", "spec v10.txt"]);
        assert_eq!(c.position, 1);
        assert_eq!(c.bases, [Basis::Token, Basis::Token]);
        assert!(!c.is_weak());
    }

    #[test]
    fn letter_and_number_revisions_order() {
        let f = [row("/d/plan rev B.docx", 1), row("/d/plan rev A.docx", 9), row("/d/plan rev AA.docx", 0)];
        let c = chain(&f, "/d/plan rev B.docx").unwrap();
        assert_eq!(names(&c), ["plan rev A.docx", "plan rev B.docx", "plan rev AA.docx"]);
        let f = [row("/d/plan r10.docx", 1), row("/d/plan r3.docx", 2)];
        assert_eq!(names(&chain(&f, "/d/plan r3.docx").unwrap()), ["plan r3.docx", "plan r10.docx"]);
    }

    #[test]
    fn dates_order_chronologically() {
        let f = [row("/d/log 20240105.txt", 1), row("/d/log 2023-12-31.txt", 2), row("/d/log 2024-02-01.txt", 0)];
        let c = chain(&f, "/d/log 2024-02-01.txt").unwrap();
        assert_eq!(names(&c), ["log 2023-12-31.txt", "log 20240105.txt", "log 2024-02-01.txt"]);
        assert_eq!(c.position, 2);
        assert!(!c.is_weak());
    }

    #[test]
    fn status_words_do_not_disturb_token_order() {
        let f = [row("/d/a v3 final.txt", 1), row("/d/a v2 draft.txt", 2)];
        let c = chain(&f, "/d/a v2 draft.txt").unwrap();
        assert_eq!(names(&c), ["a v2 draft.txt", "a v3 final.txt"]);
        assert!(!c.is_weak());
    }

    #[test]
    fn mixed_tokens_fall_back_to_mtime_and_say_so() {
        let f2 = [row("/d/spec v2.txt", 30), row("/d/spec rev B.txt", 10)];
        let c = chain(&f2, "/d/spec v2.txt").unwrap();
        assert_eq!(names(&c), ["spec rev B.txt", "spec v2.txt"]);
        assert_eq!(c.bases, [Basis::Mtime]);
        assert!(c.is_weak());
        // A member with no token at all also forces mtime.
        let f3 = [row("/d/spec.txt", 5), row("/d/spec v2.txt", 1)];
        let c = chain(&f3, "/d/spec.txt").unwrap();
        assert_eq!(names(&c), ["spec v2.txt", "spec.txt"]);
        assert_eq!(c.bases, [Basis::Mtime]);
    }

    #[test]
    fn equal_evidence_is_labelled_path_order() {
        let f = [row("/d/a v2.txt", 5), row("/d/a_v2.txt", 5)];
        let c = chain(&f, "/d/a_v2.txt").unwrap();
        assert_eq!(c.bases, [Basis::Path]);
        // Equal tokens, different mtimes: mtime breaks the tie.
        let f = [row("/d/a v2.txt", 9), row("/d/a_v2.txt", 5)];
        let c = chain(&f, "/d/a v2.txt").unwrap();
        assert_eq!(names(&c), ["a_v2.txt", "a v2.txt"]);
        assert_eq!(c.bases, [Basis::Mtime]);
    }

    #[test]
    fn unmatched_or_oversized_groups_have_no_chain() {
        let f = [row("/d/solo.txt", 1), row("/d/other.txt", 1)];
        assert!(chain(&f, "/d/solo.txt").is_none());
        // More members than MAX_VERSION_GROUP: hand-made edges, since version_edges skips such groups.
        let n = MAX_VERSION_GROUP + 1;
        let files: Vec<FileRow> = (0..n).map(|i| row(&format!("/d/a v{i}.txt"), i as i64)).collect();
        let edges: Vec<StoredEdge> = (1..n)
            .map(|i| StoredEdge {
                src: "/d/a v0.txt".into(),
                dst: format!("/d/a v{i}.txt"),
                kind: "VERSION_OF".into(),
                confidence: 0.9,
                evidence: serde_json::json!({"tokens": [["v0"], [format!("v{i}")]]}),
                class: "metadata".into(),
                created: 0,
                source_version: String::new(),
            })
            .collect();
        assert!(version_chain("/d/a v0.txt", &edges, &files).is_none());
    }

    #[test]
    fn other_edge_kinds_and_missing_files_are_tolerated() {
        let f = [row("/d/a v1.txt", 1), row("/d/a v2.txt", 2)];
        let mut edges = edges_of(&f, "/d/a v1.txt");
        edges.push(StoredEdge { kind: "SAME_ENTITY".into(), ..edges[0].clone() });
        let c = version_chain("/d/a v1.txt", &edges, &[]).unwrap();
        assert_eq!(c.bases, [Basis::Token]);
    }

    #[test]
    fn the_chain_is_the_same_from_every_member_and_every_input_order() {
        let mut f = vec![row("/d/a v3.txt", 1), row("/d/a v1.txt", 2), row("/d/a v2.txt", 3)];
        let reference = chain(&f, "/d/a v1.txt").unwrap().files;
        for path in ["/d/a v1.txt", "/d/a v2.txt", "/d/a v3.txt"] {
            let c = chain(&f, path).unwrap();
            assert_eq!(c.files, reference);
            assert_eq!(c.files[c.position], path);
        }
        f.reverse();
        let mut edges = edges_of(&f, "/d/a v2.txt");
        edges.reverse();
        assert_eq!(version_chain("/d/a v2.txt", &edges, &f).unwrap().files, reference);
    }
}
