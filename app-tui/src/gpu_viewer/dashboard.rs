//! The Toolbench dashboard: every workbench in one list, how each one opens, and what was opened last. Pure data;
//! `gui/dashboard.rs` draws it and `window.rs` carries out a launch.
//!
//! Process model (ADR-014): every workbench is its own native-window `app-tui` process (`--gpu-solid [file]`, ...).
//! The dashboard window stays open as the launcher and only records what it started (`Recents`).
use super::EditorKind;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug)]
pub struct Workbench {
    /// Stable id, kept in the recent list (`Recents`).
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    /// The native window it opens.
    pub kind: EditorKind,
}

pub const WORKBENCHES: [Workbench; 11] = [
    Workbench { id: "search", name: "Search", summary: "Keyword filters across text, Office, zip, rtf and pdf files. HTML report, CSV and JSON export.", kind: EditorKind::Search },
    Workbench { id: "fastener-hole", name: "Fastener Hole", summary: "Hole, fastener and edge-distance geometry.", kind: EditorKind::FastenerHole },
    Workbench { id: "bushing", name: "Bushing", summary: "Press fit, tolerance stack, edge-distance cross-check and verified fixes.", kind: EditorKind::Bushing },
    Workbench { id: "bushing-stackup", name: "Bushing Stackup", summary: "Line-reamed through holes and per-member bushing bore preparation.", kind: EditorKind::BushingStackup },
    Workbench { id: "pressure-vessel", name: "Pressure Vessel", summary: "Shell and head stresses, Lamé thick wall.", kind: EditorKind::PressureVessel },
    Workbench { id: "preload", name: "Preload Analysis", summary: "Fastened joint preload and member stiffness.", kind: EditorKind::Preload },
    Workbench { id: "lug", name: "Lug Analysis", summary: "Pin-loaded lug on the general FE kernel.", kind: EditorKind::Lug },
    Workbench { id: "eccentric-bushing", name: "Eccentric Bushing", summary: "Spin capacity and maximum offset of an offset bore.", kind: EditorKind::Eccentric },
    Workbench { id: "fea", name: "FEA Workbench", summary: "Sketch, STEP or mesh; supports, loads, solve, contour.", kind: EditorKind::Solid },
    Workbench { id: "members", name: "Frame and Truss", summary: "Nodes, truss and frame members, supports and loads.", kind: EditorKind::Members },
    Workbench { id: "material-lookup", name: "Material Lookup", summary: "About 1,400 materials; type to search.", kind: EditorKind::Materials },
];

impl Workbench {
    pub fn find(id: &str) -> Option<&'static Workbench> {
        WORKBENCHES.iter().find(|w| w.id == id)
    }
    /// The file used when none is given: the editor's default, in the working folder.
    pub fn default_file(&self) -> Option<PathBuf> {
        match self.kind {
            // Search keeps its saved folder unless a recent one is reopened.
            EditorKind::Search | EditorKind::Materials => None,
            kind => Some(std::env::current_dir().unwrap_or_default().join(kind.default_file())),
        }
    }
    /// Command line (after the program) that starts the workbench.
    pub fn args(&self, file: Option<&Path>) -> Vec<String> {
        let mut a = vec![self.kind.flag().to_string()];
        a.extend(file.map(|f| f.to_string_lossy().into_owned()));
        a
    }
    /// Whether the name or summary contains every word of `query` (case-insensitive); an empty query matches all.
    pub fn matches(&self, query: &str) -> bool {
        let hay = format!("{} {}", self.name, self.summary).to_lowercase();
        query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
    }
}

/// One thing the dashboard started.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recent {
    pub workbench: String,
    #[serde(default)]
    pub path: Option<String>,
    /// Seconds since the Unix epoch.
    pub opened: u64,
}

const MAX_RECENTS: usize = 12;
const FILE: &str = "recent.json";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Recents {
    /// Newest first.
    #[serde(default)]
    pub items: Vec<Recent>,
}

impl Recents {
    /// `<app data>/fea-editor/recent.json`.
    pub fn path() -> Option<PathBuf> {
        crate::paths::app_data_dir().map(|d| d.join("fea-editor").join(FILE))
    }
    /// The stored list; empty when there is no file or it cannot be read.
    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::persistence::write(path, serde_json::to_string_pretty(self).map_err(std::io::Error::other)?)
    }
    /// Put `workbench`/`path` first (an older entry for the same pair is replaced) and keep the newest few.
    pub fn record(&mut self, workbench: &str, path: Option<String>, now: u64) {
        self.items.retain(|r| !(r.workbench == workbench && r.path == path));
        self.items.insert(0, Recent { workbench: workbench.into(), path, opened: now });
        self.items.truncate(MAX_RECENTS);
    }
    /// The latest entry of a workbench.
    pub fn last(&self, workbench: &str) -> Option<&Recent> {
        self.items.iter().find(|r| r.workbench == workbench)
    }
}

/// "just now", "5 min ago", "3 h ago", "2 d ago".
pub fn ago(now: u64, then: u64) -> String {
    let s = now.saturating_sub(then);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86_400),
    }
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// What the dashboard window shows and keeps.
pub struct Dashboard {
    pub recents: Recents,
    pub query: String,
    /// The outcome of the last launch (an error says why nothing opened).
    pub message: Option<String>,
    file: Option<PathBuf>,
}

impl Dashboard {
    pub fn new(recents: Recents, file: Option<PathBuf>) -> Self {
        Self { recents, query: String::new(), message: None, file }
    }
    /// The workbenches that match the search box.
    pub fn visible(&self) -> Vec<&'static Workbench> {
        WORKBENCHES.iter().filter(|w| w.matches(&self.query)).collect()
    }
    /// Start `workbench` (on `file` when given, else its default), record it, and keep the list on disk.
    pub fn launch(&mut self, id: &str, file: Option<PathBuf>) {
        let Some(w) = Workbench::find(id) else {
            self.message = Some(format!("No workbench called {id}."));
            return;
        };
        let file = file.or_else(|| w.default_file());
        if w.kind == EditorKind::BushingStackup {
            if let Some(path)=&file {if let Err(e)=super::editor::stackup::WindowLease::acquire(path){self.message=Some(e);return;}}
        }
        match spawn(w, file.as_deref()) {
            Ok(()) => {
                self.message = Some(format!("Opened {}.", w.name));
                self.recents.record(w.id, file.map(|f| f.to_string_lossy().into_owned()), unix_now());
                if let Some(p) = &self.file {
                    if let Err(e) = self.recents.save_to(p) {
                        self.message = Some(format!("Opened {}, but the recent list could not be saved: {e}", w.name));
                    }
                }
            }
            Err(e) => self.message = Some(format!("Could not open {}: {e}", w.name)),
        }
    }
}

/// Start the workbench as a separate process that outlives nothing it needs from this one.
fn spawn(w: &Workbench, file: Option<&Path>) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut c = Command::new(&exe);
    c.args(w.args(file));
    crate::console::without_console_window(&mut c).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // The child is not waited for: the window it opens belongs to it.
    c.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_workbench_has_a_distinct_id() {
        let mut ids: Vec<_> = WORKBENCHES.iter().map(|w| w.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), WORKBENCHES.len());
        for w in WORKBENCHES {
            assert_eq!(Workbench::find(w.id).map(|f| f.name), Some(w.name));
        }
    }

    #[test]
    fn workbenches_open_their_editor_on_a_file() {
        let fea = Workbench::find("fea").unwrap();
        assert_eq!(fea.args(Some(Path::new("/x/a.json"))), ["--gpu-solid", "/x/a.json"]);
        assert_eq!(fea.args(None), ["--gpu-solid"]);
        assert_eq!(Workbench::find("search").unwrap().args(None), ["--gpu-search"]);
        assert_eq!(Workbench::find("bushing-stackup").unwrap().args(None), ["--gpu-bushing-stackup"]);
        assert!(Workbench::find("fastener-hole").unwrap().default_file().unwrap().ends_with("fastener-hole-case.json"));
        assert_eq!(Workbench::find("fastener-hole").unwrap().args(Some(Path::new("c.json"))), ["--gpu-fastener", "c.json"]);
        assert_eq!(Workbench::find("search").unwrap().default_file(), None, "Search keeps its saved folder");
        assert_eq!(Workbench::find("material-lookup").unwrap().args(None), ["--gpu-materials"]);
        assert_eq!(Workbench::find("material-lookup").unwrap().default_file(), None, "Material Lookup has no file");
        assert!(fea.default_file().unwrap().ends_with("solid-model.json"));
        let lug = Workbench::find("lug").unwrap();
        assert!(lug.default_file().unwrap().ends_with("lug-case.json"));
        assert_eq!(lug.args(None), ["--gpu-lug"]);
    }

    #[test]
    fn the_search_box_needs_every_word_in_the_name_or_summary() {
        let hay = |q: &str| WORKBENCHES.iter().filter(|w| w.matches(q)).map(|w| w.id).collect::<Vec<_>>();
        assert_eq!(hay("").len(), WORKBENCHES.len());
        assert_eq!(hay("EDGE distance"), ["fastener-hole", "bushing"]);
        assert!(hay("zzz").is_empty());
    }

    #[test]
    fn recents_keep_the_newest_first_one_per_file_and_a_bounded_list() {
        let mut r = Recents::default();
        r.record("fea", Some("a.json".into()), 10);
        r.record("lug", None, 20);
        r.record("fea", Some("a.json".into()), 30);
        assert_eq!(r.items.iter().map(|i| (i.workbench.as_str(), i.opened)).collect::<Vec<_>>(), [("fea", 30), ("lug", 20)]);
        r.record("fea", Some("b.json".into()), 40);
        assert_eq!(r.last("fea").unwrap().path.as_deref(), Some("b.json"));
        assert_eq!(r.items.len(), 3);
        for k in 0..30 {
            r.record("fea", Some(format!("f{k}")), 50 + k);
        }
        assert_eq!(r.items.len(), MAX_RECENTS);
        assert_eq!(r.last("lug"), None, "the oldest fell off");
    }

    #[test]
    fn recents_round_trip_and_a_missing_or_broken_file_is_an_empty_list() {
        let dir = std::env::temp_dir().join(format!("dashboard-recents-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("fea-editor").join(FILE);
        assert_eq!(Recents::load_from(&path), Recents::default());
        let mut r = Recents::default();
        r.record("fea", Some("a.json".into()), 5);
        r.save_to(&path).unwrap();
        assert_eq!(Recents::load_from(&path), r);
        crate::persistence::write(&path, "{ nope").unwrap();
        assert_eq!(Recents::load_from(&path), Recents::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ages_read_in_the_largest_whole_unit() {
        assert_eq!(ago(100, 100), "just now");
        assert_eq!(ago(1000, 700), "5 min ago");
        assert_eq!(ago(10_000, 100), "2 h ago");
        assert_eq!(ago(300_000, 100), "3 d ago");
        assert_eq!(ago(5, 100), "just now", "a clock that went backwards");
    }
}
