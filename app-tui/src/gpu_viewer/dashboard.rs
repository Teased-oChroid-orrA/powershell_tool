//! The Toolbench dashboard: every workbench in one list, how each one opens, and what was opened last. Pure data;
//! `gui/dashboard.rs` draws it and `window.rs` carries out a launch.
//!
//! Process model (ADR-014): a workbench with a native window is its own `app-tui` process (`--gpu-solid [file]`, ...);
//! one that is still a terminal toolbox is opened in a new terminal (`app-tui --toolbox <slug>`). The dashboard window
//! stays open as the launcher and only records what it started (`Recents`).
use super::EditorKind;
use crate::nav::ToolId;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How a workbench opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opens {
    /// A native window process.
    Window(EditorKind),
    /// A terminal toolbox.
    Terminal(ToolId),
}

#[derive(Clone, Copy, Debug)]
pub struct Workbench {
    /// Stable id: the toolbox slug, or the editor's for the ones without a terminal toolbox.
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub opens: Opens,
}

pub const WORKBENCHES: [Workbench; 10] = [
    Workbench { id: "search", name: "Search", summary: "Keyword filters across text, Office, zip, rtf and pdf files. HTML report, CSV and JSON export.", opens: Opens::Terminal(ToolId::Search) },
    Workbench { id: "fastener-hole", name: "Fastener Hole", summary: "Hole, fastener and edge-distance geometry.", opens: Opens::Terminal(ToolId::FastenerHole) },
    Workbench { id: "bushing", name: "Bushing", summary: "Press fit, tolerance stack, edge-distance cross-check and verified fixes.", opens: Opens::Window(EditorKind::Bushing) },
    Workbench { id: "pressure-vessel", name: "Pressure Vessel", summary: "Shell and head stresses, Lamé thick wall.", opens: Opens::Terminal(ToolId::PressureVessel) },
    Workbench { id: "preload", name: "Preload Analysis", summary: "Fastened joint preload and member stiffness.", opens: Opens::Terminal(ToolId::PreloadAnalysis) },
    Workbench { id: "lug", name: "Lug Analysis", summary: "Pin-loaded lug on the general FE kernel.", opens: Opens::Terminal(ToolId::LugAnalysis) },
    Workbench { id: "eccentric-bushing", name: "Eccentric Bushing", summary: "Spin capacity and maximum offset of an offset bore.", opens: Opens::Window(EditorKind::Eccentric) },
    Workbench { id: "fea", name: "FEA Workbench", summary: "Sketch, STEP or mesh; supports, loads, solve, contour.", opens: Opens::Window(EditorKind::Solid) },
    Workbench { id: "members", name: "Frame and Truss", summary: "Nodes, truss and frame members, supports and loads.", opens: Opens::Window(EditorKind::Members) },
    Workbench { id: "material-lookup", name: "Material Lookup", summary: "About 1,400 materials; type to search.", opens: Opens::Terminal(ToolId::MaterialLookup) },
];

impl Workbench {
    pub fn find(id: &str) -> Option<&'static Workbench> {
        WORKBENCHES.iter().find(|w| w.id == id)
    }
    pub fn is_window(&self) -> bool {
        matches!(self.opens, Opens::Window(_))
    }
    /// The file used when none is given: the editor's default, in the working folder. Terminal toolboxes keep their own.
    pub fn default_file(&self) -> Option<PathBuf> {
        match self.opens {
            Opens::Window(kind) => Some(std::env::current_dir().unwrap_or_default().join(kind.default_file())),
            Opens::Terminal(_) => None,
        }
    }
    /// Command line (after the program) that starts the workbench.
    pub fn args(&self, file: Option<&Path>) -> Vec<String> {
        match self.opens {
            Opens::Window(kind) => {
                let mut a = vec![kind.flag().to_string()];
                a.extend(file.map(|f| f.to_string_lossy().into_owned()));
                a
            }
            Opens::Terminal(tool) => vec!["--toolbox".into(), tool.slug().into()],
        }
    }
    /// Whether the name or summary contains every word of `query` (case-insensitive); an empty query matches all.
    pub fn matches(&self, query: &str) -> bool {
        let hay = format!("{} {}", self.name, self.summary).to_lowercase();
        query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
    }
}

/// Program and arguments that run `exe args` in a new terminal window on `os` (`std::env::consts::OS`), or `None`
/// where no terminal can be started without writing a script (macOS: see [`macos_script`]).
pub fn terminal_command(os: &str, exe: &Path, args: &[String]) -> Option<(String, Vec<String>)> {
    let exe = exe.to_string_lossy().into_owned();
    match os {
        // `start` opens a new console window for a console program; the empty string is the window title.
        "windows" => Some(("cmd".into(), ["/C", "start", "", &exe].iter().map(|s| s.to_string()).chain(args.iter().cloned()).collect())),
        "linux" => Some(("x-terminal-emulator".into(), ["-e".to_string(), exe].into_iter().chain(args.iter().cloned()).collect())),
        _ => None,
    }
}

/// A `.command` script that runs `exe args`; macOS opens it in Terminal.
pub fn macos_script(exe: &Path, args: &[String]) -> String {
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let mut line = quote(&exe.to_string_lossy());
    for a in args {
        line.push(' ');
        line.push_str(&quote(a));
    }
    format!("#!/bin/sh\nexec {line}\n")
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
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(std::io::Error::other)?)
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
    /// Start `workbench` (on `file` when it is a window workbench with one), record it, and keep the list on disk.
    pub fn launch(&mut self, id: &str, file: Option<PathBuf>) {
        let Some(w) = Workbench::find(id) else {
            self.message = Some(format!("No workbench called {id}."));
            return;
        };
        let file = file.or_else(|| w.default_file());
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
    let args = w.args(file);
    let quiet = |c: &mut Command| {
        c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    };
    if w.is_window() {
        let mut c = Command::new(&exe);
        c.args(&args);
        quiet(&mut c);
        // The child is not waited for: the window it opens belongs to it.
        return c.spawn().map(|_| ()).map_err(|e| e.to_string());
    }
    let os = std::env::consts::OS;
    if let Some((program, a)) = terminal_command(os, &exe, &args) {
        let mut c = Command::new(&program);
        c.args(&a);
        quiet(&mut c);
        return c.spawn().map(|_| ()).map_err(|e| format!("{program}: {e}"));
    }
    if os == "macos" {
        return open_macos_script(w, &exe, &args);
    }
    Err(format!("no terminal is known for {os}; run app-tui in a terminal"))
}

/// macOS: Terminal runs a `.command` script (the only way to hand it arguments). Unix-only code: the Windows build must still compile.
#[cfg(unix)]
fn open_macos_script(w: &Workbench, exe: &Path, args: &[String]) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?.keep();
    let script = dir.join(format!("toolbench-{}.command", w.id));
    std::fs::write(&script, macos_script(exe, args)).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    let mut c = Command::new("open");
    c.arg(&script).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    c.spawn().map(|_| ()).map_err(|e| format!("open: {e}"))
}

#[cfg(not(unix))]
fn open_macos_script(_: &Workbench, _: &Path, _: &[String]) -> Result<(), String> {
    Err("this build has no macOS terminal launcher".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_workbench_has_a_distinct_id_and_the_terminal_ones_use_their_toolbox_slug() {
        let mut ids: Vec<_> = WORKBENCHES.iter().map(|w| w.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), WORKBENCHES.len());
        for w in WORKBENCHES {
            if let Opens::Terminal(t) = w.opens {
                assert_eq!(w.id, t.slug(), "{}", w.name);
                assert!(t.enabled());
            }
            assert_eq!(Workbench::find(w.id).map(|f| f.name), Some(w.name));
        }
    }

    #[test]
    fn window_workbenches_open_their_editor_on_a_file_and_terminal_ones_by_slug() {
        let fea = Workbench::find("fea").unwrap();
        assert_eq!(fea.args(Some(Path::new("/x/a.json"))), ["--gpu-solid", "/x/a.json"]);
        assert_eq!(fea.args(None), ["--gpu-solid"]);
        assert_eq!(Workbench::find("search").unwrap().args(None), ["--toolbox", "search"]);
        assert!(fea.default_file().unwrap().ends_with("solid-model.json"));
        assert_eq!(Workbench::find("lug").unwrap().default_file(), None);
    }

    #[test]
    fn the_search_box_needs_every_word_in_the_name_or_summary() {
        let hay = |q: &str| WORKBENCHES.iter().filter(|w| w.matches(q)).map(|w| w.id).collect::<Vec<_>>();
        assert_eq!(hay("").len(), WORKBENCHES.len());
        assert_eq!(hay("EDGE distance"), ["fastener-hole", "bushing"]);
        assert!(hay("zzz").is_empty());
    }

    #[test]
    fn a_terminal_is_started_the_way_each_os_does_it() {
        let (p, a) = terminal_command("windows", Path::new(r"C:\t\app-tui.exe"), &["--toolbox".into(), "lug".into()]).unwrap();
        assert_eq!((p.as_str(), a.as_slice()), ("cmd", ["/C", "start", "", r"C:\t\app-tui.exe", "--toolbox", "lug"].map(String::from).as_slice()));
        assert_eq!(terminal_command("linux", Path::new("/a"), &["x".into()]).unwrap().1, ["-e", "/a", "x"]);
        assert!(terminal_command("macos", Path::new("/a"), &[]).is_none());
        let s = macos_script(Path::new("/Apps/it's/app-tui"), &["--toolbox".into(), "lug".into()]);
        assert_eq!(s, "#!/bin/sh\nexec '/Apps/it'\\''s/app-tui' '--toolbox' 'lug'\n");
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
        std::fs::write(&path, "{ nope").unwrap();
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
