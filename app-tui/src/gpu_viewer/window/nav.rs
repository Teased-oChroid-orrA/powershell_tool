//! The dashboard window as a shell (ADR-024): workbenches open in this window, stay parked with their state while
//! another one or the dashboard is shown, and are closed from the nav strip. A standalone `--gpu-*` window has no `Nav`.
use super::*;
use crate::gpu_viewer::{
    dashboard::{Dashboard, Workbench},
    editor::stackup::WindowLease,
    nav::{NavAct, NavStrip, NavTab},
};
use std::path::PathBuf;

/// What the dashboard window holds besides the one thing it shows.
#[derive(Default)]
pub(super) struct Nav {
    /// The dashboard while a workbench is shown.
    home: Option<Dashboard>,
    /// Open workbenches in tab order.
    open: Vec<Open>,
    /// The workbench shown; `None` is the dashboard.
    active: Option<&'static str>,
    /// A close waiting for "Keep open" / "Discard and close".
    ask_close: Option<&'static str>,
}

struct Open {
    id: &'static str,
    /// Its state while it is not the one shown (the shown one lives in the viewer's own fields).
    parked: Option<Parked>,
    /// Keeps a case that only one window may hold (the stackup) for as long as the tab is open.
    #[allow(dead_code)]
    lease: Option<WindowLease>,
}

struct Parked {
    editor: Box<dyn Authoring>,
    project: ViewerProject,
    view: ViewState,
}

impl Viewer {
    fn editor_of(&self, id: &str) -> Option<&dyn Authoring> {
        let nav = self.nav.as_ref()?;
        if nav.active == Some(id) {
            return self.editor.as_deref();
        }
        nav.open.iter().find(|o| o.id == id)?.parked.as_ref().map(|p| &*p.editor)
    }
    /// The dashboard, wherever it is parked.
    fn home_mut(&mut self) -> Option<&mut Dashboard> {
        match &mut self.dashboard {
            Some(d) => Some(d),
            None => self.nav.as_mut()?.home.as_mut(),
        }
    }
    /// Say something where the person is looking: the dashboard's line, else the shown workbench's status.
    fn tell(&mut self, text: String) {
        if let Some(d) = &mut self.dashboard {
            d.message = Some(text);
        } else if let Some(ed) = &mut self.editor {
            ed.set_status(text);
        } else {
            self.error = Some(text);
        }
    }

    /// What the nav strip shows; `None` for a standalone window and while nothing is open.
    pub(super) fn nav_strip(&self) -> Option<NavStrip> {
        let nav = self.nav.as_ref().filter(|n| !n.open.is_empty())?;
        let tabs: Vec<NavTab> = nav
            .open
            .iter()
            .filter_map(|o| {
                let ed = self.editor_of(o.id)?;
                Some(NavTab { id: o.id, name: Workbench::find(o.id)?.name, dirty: ed.dirty(), busy: ed.solving_for().is_some() })
            })
            .collect();
        let ask_close = nav.ask_close.filter(|id| tabs.iter().any(|t| t.id == *id));
        Some(NavStrip { tabs, active: nav.active, ask_close })
    }
    /// The workbench shown in the dashboard window; `None` for the dashboard (and a standalone window).
    #[cfg(test)]
    pub(super) fn shown_workbench(&self) -> Option<&'static str> {
        self.nav.as_ref()?.active
    }
    /// The dashboard window with at least one workbench open.
    pub(super) fn has_open_workbench(&self) -> bool {
        self.nav.as_ref().is_some_and(|n| !n.open.is_empty())
    }
    /// A parked workbench's background work finished (or its files changed): redraw to show it.
    pub(super) fn poll_parked(&mut self) -> bool {
        let Some(nav) = &mut self.nav else { return false };
        let mut changed = false;
        for p in nav.open.iter_mut().filter_map(|o| o.parked.as_mut()) {
            changed |= p.editor.poll();
        }
        changed
    }
    /// Some tab is solving: the strip's busy mark needs a steady redraw.
    pub(super) fn any_tab_busy(&self) -> bool {
        let Some(nav) = &self.nav else { return false };
        nav.open.iter().any(|o| self.editor_of(o.id).is_some_and(|e| e.solving_for().is_some()))
    }
    /// Every open workbench's editor, the shown one first (recovery snapshots cover the parked ones too).
    pub(super) fn editors_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn Authoring>> {
        let parked = self.nav.iter_mut().flat_map(|n| n.open.iter_mut()).filter_map(|o| o.parked.as_mut()).map(|p| &mut p.editor);
        self.editor.iter_mut().chain(parked)
    }

    pub(super) fn nav_act(&mut self, act: NavAct) {
        match act {
            NavAct::Home => self.show(None),
            NavAct::Open(id) => self.open_workbench(id, None),
            NavAct::Close(id) => self.close_tab(id, false),
            NavAct::Discard(id) => self.close_tab(id, true),
            NavAct::KeepOpen => {
                if let Some(nav) = &mut self.nav {
                    nav.ask_close = None;
                }
            }
            NavAct::PopOut(id) => self.pop_out(id),
        }
    }

    /// Show workbench `id` in this window: its tab when open (on `file` when that is a different file and the tab
    /// holds no unsaved edits), else a new one. Nothing changes when the workbench fails to open.
    pub(super) fn open_workbench(&mut self, id: &str, file: Option<PathBuf>) {
        let Some(w) = Workbench::find(id) else { return };
        if self.nav.is_none() {
            return;
        }
        let path = file.clone().or_else(|| w.default_file()).unwrap_or_else(|| PathBuf::from(w.kind.default_file()));
        if let Some(ed) = self.editor_of(w.id) {
            if file.is_none() || ed.path() == path {
                return self.show(Some(w.id));
            }
            if ed.dirty() {
                self.show(Some(w.id));
                self.tell(format!("{} has unsaved edits: save or close it before opening another file.", w.name));
                return;
            }
            // A clean tab is replaced: close it first so a case lease is free for the new one.
            self.close_tab(w.id, true);
        }
        let built = match build_workbench(w.kind, &path) {
            Ok(b) => b,
            Err(e) => return self.tell(format!("Could not open {}: {e}", w.name)),
        };
        self.park_current();
        let display = self.view.display_settings();
        self.project = ViewerProject::single(built.scene);
        self.view = fresh_view(&self.project, Some(&*built.editor));
        self.view.set_display(display);
        self.editor = Some(built.editor);
        if let Some(nav) = &mut self.nav {
            nav.open.push(Open { id: w.id, parked: None, lease: built.lease });
            nav.active = Some(w.id);
        }
        if let Some(home) = self.home_mut() {
            home.note_opened(w, file.or_else(|| w.default_file()));
        }
        self.after_swap();
    }

    /// Show `to` (a workbench that is open, or the dashboard) and park what was shown.
    fn show(&mut self, to: Option<&str>) {
        let to = to.and_then(|id| self.nav.as_ref()?.open.iter().find(|o| o.id == id).map(|o| o.id));
        if self.nav.as_ref().is_none_or(|n| n.active == to) {
            return;
        }
        self.park_current();
        let display = self.view.display_settings();
        let Some(nav) = &mut self.nav else { return };
        let parked = to.and_then(|id| nav.open.iter_mut().find(|o| o.id == id)?.parked.take());
        match parked {
            Some(p) => {
                self.editor = Some(p.editor);
                self.project = p.project;
                self.view = p.view;
                self.view.set_display(display);
                nav.active = to;
            }
            None => {
                self.dashboard = nav.home.take();
                nav.active = None;
            }
        }
        self.after_swap();
    }

    /// Move what is shown (the dashboard, or a workbench with its model and view) out of the viewer's own fields.
    fn park_current(&mut self) {
        let blank = ViewerProject::single(blank_scene("dashboard"));
        let mut view = fresh_view(&blank, None);
        view.set_display(self.view.display_settings());
        let Some(nav) = &mut self.nav else { return };
        match nav.active.take() {
            None => nav.home = self.dashboard.take(),
            Some(id) => {
                if let (Some(editor), Some(open)) = (self.editor.take(), nav.open.iter_mut().find(|o| o.id == id)) {
                    open.parked = Some(Parked {
                        editor,
                        project: std::mem::replace(&mut self.project, blank),
                        view: std::mem::replace(&mut self.view, view),
                    });
                }
            }
        }
    }

    /// The shown thing changed: hand its scene to the renderer and drop the previous page's transient state.
    fn after_swap(&mut self) {
        if let Some(r) = &mut self.renderer {
            if let Err(e) = r.set_scene(&self.project.scenes[self.project.selected]) {
                self.error = Some(e);
            }
        }
        self.gui.reset_page();
        self.pointer = Pointer::default();
        (self.egui_press, self.grabbing, self.pan_gesture) = (false, false, None);
        self.quit_warned = false;
        self.next_redraw = Instant::now();
    }

    /// Close `id`'s tab. While it holds unsaved edits or a running solve the strip asks first, unless `force`.
    fn close_tab(&mut self, id: &str, force: bool) {
        let Some(ed) = self.editor_of(id) else { return };
        if (ed.dirty() || ed.solving_for().is_some()) && !force {
            if let Some(nav) = &mut self.nav {
                nav.ask_close = Workbench::find(id).map(|w| w.id);
            }
            return;
        }
        if self.nav.as_ref().is_some_and(|n| n.active == Some(id)) {
            self.show(None);
        }
        if let Some(nav) = &mut self.nav {
            nav.open.retain(|o| o.id != id);
            nav.ask_close = None;
        }
    }

    /// Move the workbench into a window of its own; it opens again here if that window cannot start.
    fn pop_out(&mut self, id: &str) {
        let Some(w) = Workbench::find(id) else { return };
        let Some(ed) = self.editor_of(id) else { return };
        if ed.dirty() || ed.solving_for().is_some() {
            return self.tell(format!("Save {} and let its solve finish before moving it to a new window.", w.name));
        }
        let file = w.default_file().is_some().then(|| ed.path().to_path_buf());
        self.close_tab(w.id, true);
        let started = self.home_mut().is_some_and(|d| d.launch(w.id, file.clone()));
        if started {
            self.show(None);
        } else {
            self.open_workbench(w.id, file);
        }
    }

    /// The first workbench with unsaved edits, the shown one first.
    fn dirty_tab(&self) -> Option<&'static str> {
        let nav = self.nav.as_ref()?;
        let is_dirty = |id| self.editor_of(id).is_some_and(|e| e.dirty());
        nav.active.filter(|id| is_dirty(*id)).or_else(|| nav.open.iter().map(|o| o.id).find(|id| is_dirty(*id)))
    }
    /// Quit: bring a workbench with unsaved edits forward and ask about it before anything closes.
    pub(super) fn request_quit(&mut self) {
        if let Some(id) = self.dirty_tab() {
            self.show(Some(id));
        }
        match &mut self.editor {
            Some(ed) if ed.dirty() => {
                self.quit_warned = true;
                self.gui.confirm_close = true;
                ed.set_status("Unsaved changes: choose Save and close, Discard changes, or Keep editing.".into());
            }
            _ => self.quit = true,
        }
    }
    /// "Discard changes" in the close question: drop this workbench; quit once none is left to ask about.
    pub(super) fn discard_and_close(&mut self) {
        if let Some(id) = self.nav.as_ref().and_then(|n| n.active) {
            self.close_tab(id, true);
            self.gui.confirm_close = false;
            self.quit_warned = false;
            if self.dirty_tab().is_some() {
                return self.request_quit();
            }
        }
        self.quit = true;
    }
}
