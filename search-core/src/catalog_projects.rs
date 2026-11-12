//! Project discovery (#240 P3-2): folders whose files form one connected cluster, offered as
//! *suggestions* the user may scope a search to. Nothing here changes anything; it reads stored
//! relationship edges and file paths and returns evidence-bearing candidates.
//!
//! A candidate is a folder strictly below a registered root with at least [`MIN_FILES`] files in its
//! subtree whose largest connected component (files linked by `VERSION_OF` edges or `SAME_ENTITY`
//! edges of at least [`MIN_CONFIDENCE`]) holds at least [`MIN_COHESION`] of those files. Linking by
//! name (versions) and by shared identifying strings (part numbers, standards) is what makes files
//! "about the same thing"; duplicates are not used because copies cross project boundaries. A folder
//! that links several unrelated groups fails the cohesion test, so discovery descends into it; a
//! folder is skipped when its single child folder holds the same files (the child is the project).
//! Top-down: once a folder is chosen, nothing below it is offered. The evidence is the shared
//! identifiers that link most files, as stored, never generated. The numbers are heuristics, not
//! probabilities; precision on a labelled tree is recorded in `docs/knowledge-engine-audit.md`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::catalog::Catalog;
use crate::catalog_edges::{parent_folder, StoredEdge};

/// Fewest files a project folder needs below it.
pub const MIN_FILES: usize = 3;
/// Share of a folder's files that must sit in its largest linked component.
pub const MIN_COHESION: f64 = 0.6;
/// Weakest `SAME_ENTITY` edge that links two files: an identifier held by at most six files links them
/// (lower than the preview's 0.5 bar, because here many weak links add up to one cluster).
pub const MIN_CONFIDENCE: f64 = 0.19;
/// Most suggestions returned (largest clusters first).
pub const MAX_SUGGESTIONS: usize = 12;
const MAX_EVIDENCE: usize = 3;

/// A folder that looks like one project, with the evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectSuggestion {
    pub folder: String,
    /// Files in the folder's subtree.
    pub files: usize,
    /// Files in the largest linked component.
    pub linked: usize,
    /// `linked / files`.
    pub cohesion: f64,
    /// Identifiers shared by the linked files (as stored), most linking first, and a count of versions.
    pub evidence: Vec<String>,
}

fn under(path: &str, folder: &str) -> bool {
    path.len() > folder.len() && path.starts_with(folder) && matches!(path.as_bytes()[folder.len()], b'/' | b'\\')
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// Suggested projects from catalogued `files`, the `roots` they belong to and the stored `edges`
/// (other edge types are ignored). Deterministic.
pub fn discover(files: &[String], roots: &[String], edges: &[StoredEdge]) -> Vec<ProjectSuggestion> {
    let index: HashMap<&str, usize> = files.iter().enumerate().map(|(i, p)| (p.as_str(), i)).collect();
    // Subtree membership per candidate folder.
    let mut members: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, path) in files.iter().enumerate() {
        let Some(root) = roots.iter().filter(|r| under(path, r)).max_by_key(|r| r.len()) else { continue };
        let mut folder = parent_folder(path);
        while folder != root.as_str() && under(folder, root) {
            members.entry(folder.to_string()).or_default().push(i);
            folder = parent_folder(folder);
        }
    }
    // A folder whose only content is one child folder is that child.
    let redundant: BTreeSet<String> = members
        .iter()
        .filter_map(|(f, m)| {
            let parent = parent_folder(f);
            members.get(parent).filter(|p| p.len() == m.len()).map(|_| parent.to_string())
        })
        .collect();

    let linking: Vec<&StoredEdge> = edges
        .iter()
        .filter(|e| e.kind == "VERSION_OF" || (e.kind == "SAME_ENTITY" && e.confidence >= MIN_CONFIDENCE))
        .collect();
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); files.len()];
    for (k, e) in linking.iter().enumerate() {
        if let (Some(&a), Some(&b)) = (index.get(e.src.as_str()), index.get(e.dst.as_str())) {
            adjacency[a].push(k);
            adjacency[b].push(k);
        }
    }

    let mut chosen: Vec<ProjectSuggestion> = Vec::new();
    // Sorted order puts every folder before the folders below it.
    for (folder, list) in &members {
        let n = list.len();
        if n < MIN_FILES || redundant.contains(folder) || chosen.iter().any(|c| under(folder, &c.folder)) {
            continue;
        }
        let inside = |p: &str| under(p, folder);
        let mut parent: Vec<usize> = (0..files.len()).collect();
        let mut used: Vec<usize> = Vec::new();
        for &a in list {
            for &k in &adjacency[a] {
                let e = linking[k];
                let (Some(&x), Some(&y)) = (index.get(e.src.as_str()), index.get(e.dst.as_str())) else { continue };
                if inside(&files[x]) && inside(&files[y]) && a == x {
                    let (rx, ry) = (find(&mut parent, x), find(&mut parent, y));
                    parent[rx] = ry;
                    used.push(k);
                }
            }
        }
        let mut sizes: HashMap<usize, usize> = HashMap::new();
        for &a in list {
            *sizes.entry(find(&mut parent, a)).or_default() += 1;
        }
        let Some((&top, &linked)) = sizes.iter().max_by_key(|(r, s)| (**s, std::cmp::Reverse(**r))) else { continue };
        let cohesion = linked as f64 / n as f64;
        if linked < MIN_FILES || cohesion < MIN_COHESION {
            continue;
        }
        let evidence = evidence_of(&used, &linking, files, &index, &mut parent, top);
        chosen.push(ProjectSuggestion { folder: folder.clone(), files: n, linked, cohesion, evidence });
    }
    chosen.sort_by(|a, b| b.linked.cmp(&a.linked).then_with(|| a.folder.cmp(&b.folder)));
    chosen.truncate(MAX_SUGGESTIONS);
    chosen
}

/// The identifiers on the edges of the linked component, most edges first, plus a version count.
fn evidence_of(
    used: &[usize],
    linking: &[&StoredEdge],
    files: &[String],
    index: &HashMap<&str, usize>,
    parent: &mut [usize],
    top: usize,
) -> Vec<String> {
    let mut by_text: BTreeMap<String, usize> = BTreeMap::new();
    let mut versions = BTreeSet::new();
    for &k in used {
        let e = linking[k];
        let (Some(&x), Some(&y)) = (index.get(e.src.as_str()), index.get(e.dst.as_str())) else { continue };
        if find(parent, x) != top {
            continue;
        }
        if e.kind == "VERSION_OF" {
            versions.insert(&files[x]);
            versions.insert(&files[y]);
        } else if let Some(shared) = e.evidence["shared"].as_array() {
            for s in shared {
                if let Some(text) = s["src_text"].as_str().filter(|t| !t.is_empty()) {
                    *by_text.entry(text.to_string()).or_default() += 1;
                }
            }
        }
    }
    let mut ranked: Vec<(String, usize)> = by_text.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut out: Vec<String> = ranked.into_iter().take(MAX_EVIDENCE).map(|(t, _)| t).collect();
    if !versions.is_empty() {
        out.push(format!("{} versioned files", versions.len()));
    }
    out
}

impl Catalog {
    /// [`discover`] over this catalog's files, roots and stored edges. Reads only.
    pub fn project_suggestions(&self) -> rusqlite::Result<Vec<ProjectSuggestion>> {
        let mut seen = BTreeSet::new();
        let files: Vec<String> = self.files()?.into_iter().map(|f| f.path).filter(|p| seen.insert(p.clone())).collect();
        let roots: Vec<String> = self.roots()?.into_iter().map(|r| r.path).collect();
        let edges = self.query_edges(
            "WHERE type='VERSION_OF' OR (type='SAME_ENTITY' AND confidence>=?1)",
            rusqlite::params![MIN_CONFIDENCE],
        )?;
        Ok(discover(&files, &roots, &edges))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entity_edge(a: &str, b: &str, text: &str) -> StoredEdge {
        StoredEdge {
            src: a.into(),
            dst: b.into(),
            kind: "SAME_ENTITY".into(),
            confidence: 0.9,
            evidence: json!({"shared_count": 1, "shared": [{"src_text": text, "dst_text": text}]}),
            class: "statistical".into(),
            created: 0,
            source_version: String::new(),
        }
    }

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn linked_folder_is_suggested_with_its_identifier() {
        let files = paths(&["/r/a/1.txt", "/r/a/2.txt", "/r/a/3.txt", "/r/misc/x.txt", "/r/misc/y.txt", "/r/misc/z.txt"]);
        let edges = vec![entity_edge("/r/a/1.txt", "/r/a/2.txt", "PN-100"), entity_edge("/r/a/2.txt", "/r/a/3.txt", "PN-100")];
        let out = discover(&files, &["/r".into()], &edges);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!((out[0].folder.as_str(), out[0].files, out[0].linked), ("/r/a", 3, 3));
        assert_eq!(out[0].evidence, vec!["PN-100".to_string()]);
    }

    #[test]
    fn two_unrelated_projects_are_two_suggestions_not_their_parent() {
        let files = paths(&["/r/p/a/1", "/r/p/a/2", "/r/p/a/3", "/r/p/b/1", "/r/p/b/2", "/r/p/b/3"]);
        let edges = vec![
            entity_edge("/r/p/a/1", "/r/p/a/2", "A-1"),
            entity_edge("/r/p/a/2", "/r/p/a/3", "A-1"),
            entity_edge("/r/p/b/1", "/r/p/b/2", "B-1"),
            entity_edge("/r/p/b/2", "/r/p/b/3", "B-1"),
        ];
        let out = discover(&files, &["/r".into()], &edges);
        let folders: Vec<&str> = out.iter().map(|s| s.folder.as_str()).collect();
        assert_eq!(folders, vec!["/r/p/a", "/r/p/b"]);
    }

    #[test]
    fn a_wrapper_with_one_child_offers_the_child_and_the_root_is_never_offered() {
        let files = paths(&["/r/w/docs/1", "/r/w/docs/2", "/r/w/docs/3"]);
        let edges = vec![entity_edge("/r/w/docs/1", "/r/w/docs/2", "Q"), entity_edge("/r/w/docs/2", "/r/w/docs/3", "Q")];
        let out = discover(&files, &["/r".into()], &edges);
        assert_eq!(out.iter().map(|s| s.folder.as_str()).collect::<Vec<_>>(), vec!["/r/w/docs"]);
    }

    #[test]
    fn weak_edges_small_folders_and_unrooted_files_are_not_projects() {
        let mut weak = entity_edge("/r/a/1", "/r/a/2", "Z");
        weak.confidence = 0.2;
        let files = paths(&["/r/a/1", "/r/a/2", "/r/a/3", "/r/b/1", "/r/b/2", "/o/c/1", "/o/c/2", "/o/c/3"]);
        let edges = vec![weak, entity_edge("/r/b/1", "/r/b/2", "Y"), entity_edge("/o/c/1", "/o/c/2", "X"), entity_edge("/o/c/2", "/o/c/3", "X")];
        assert!(discover(&files, &["/r".into()], &edges).is_empty());
    }

    #[test]
    fn windows_separators_work_and_versions_count_as_links() {
        let files = paths(&[r"C:\r\a\spec_v1.docx", r"C:\r\a\spec_v2.docx", r"C:\r\a\spec_v3.docx"]);
        let v = |a: &str, b: &str| StoredEdge { kind: "VERSION_OF".into(), confidence: 0.9, evidence: json!({}), ..entity_edge(a, b, "") };
        let edges = vec![v(&files[0], &files[1]), v(&files[1], &files[2])];
        let out = discover(&files, &[r"C:\r".into()], &edges);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].evidence, vec!["3 versioned files".to_string()]);
    }
}
