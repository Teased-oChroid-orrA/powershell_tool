//! Pointer handling over the viewport: model clicks, grabs, box select, context menu, pan and zoom.
use super::*;

impl Viewer {
    /// The viewport-relative pixel of a window pixel, if it is inside the viewport and not under a panel or popup.
    pub(super) fn viewport_point(&self, at: (f64, f64)) -> Option<(f64, f64)> {
        let v = self.viewport();
        (v.contains(at.0, at.1) && !self.gui.wants_pointer())
            .then_some((at.0 - v.x as f64, at.1 - v.y as f64))
    }
    /// A press released without dragging: in the editor it is a tool click on the model.
    pub(super) fn model_click(&mut self, at: (f64, f64)) {
        let Some(local) = self.viewport_point(at) else {
            return;
        };
        let vp = self.viewport();
        let cam = self.camera();
        let additive = self.mods.shift_key();
        if let Some(ed) = &mut self.editor {
            ed.click_with(
                [local.0 as f32, local.1 as f32],
                [vp.w as f32, vp.h as f32],
                &cam,
                additive,
            );
            if let Some(items) = ed.take_popup() {
                self.gui.open_context(at, items);
            }
        }
    }
    /// Left press over the model: does the editor take the gesture (drag a sketch point)?
    pub(super) fn begin_grab(&mut self, at: (f64, f64)) -> bool {
        let Some(local) = self.viewport_point(at) else {
            return false;
        };
        let vp = self.viewport();
        let cam = self.camera();
        self.editor.as_mut().is_some_and(|ed| {
            ed.grab(
                [local.0 as f32, local.1 as f32],
                [vp.w as f32, vp.h as f32],
                &cam,
            )
        })
    }
    /// Cursor moved: a grabbed entity follows it, otherwise the editor may show a hover preview.
    pub(super) fn editor_pointer(&mut self, at: (f64, f64)) {
        if self.editor.is_none() || self.gui.floating_open() {
            return;
        }
        let vp = self.viewport();
        let cam = self.camera();
        let grabbing = self.grabbing;
        let Some(local) = self
            .viewport_point(at)
            .or_else(|| grabbing.then_some((at.0 - vp.x as f64, at.1 - vp.y as f64)))
        else {
            return;
        };
        let (px, size) = ([local.0 as f32, local.1 as f32], [vp.w as f32, vp.h as f32]);
        if let Some(ed) = &mut self.editor {
            if grabbing {
                ed.drag(px, size, &cam);
            } else {
                ed.hover(px, size, &cam);
            }
        }
    }
    /// A Shift-drag released over the viewport: box-select in the editor (Ctrl/Cmd adds to the selection).
    pub(super) fn model_box(&mut self, a: (f64, f64), b: (f64, f64)) {
        let vp = self.viewport();
        let local = |p: (f64, f64)| {
            [
                (p.0 - vp.x as f64).clamp(0.0, vp.w as f64) as f32,
                (p.1 - vp.y as f64).clamp(0.0, vp.h as f64) as f32,
            ]
        };
        let additive = self.mods.control_key() || self.mods.super_key();
        let cam = self.camera();
        if let Some(ed) = &mut self.editor {
            ed.box_select(
                local(a),
                local(b),
                [vp.w as f32, vp.h as f32],
                &cam,
                additive,
            );
        }
    }
    /// Right press in the viewport: select what is under the cursor and open the context menu there.
    pub(super) fn open_context_menu(&mut self, at: (f64, f64)) {
        let Some(local) = self.viewport_point(at) else {
            return;
        };
        let vp = self.viewport();
        let cam = self.camera();
        if let Some(ed) = &mut self.editor {
            ed.select_at(
                [local.0 as f32, local.1 as f32],
                [vp.w as f32, vp.h as f32],
                &cam,
            );
        }
        let items = menus::context(&self.menu_context());
        self.gui.open_context(at, items);
    }
    /// Shift the view by a cursor offset in window pixels.
    pub(super) fn pan_by_pixels(&mut self, (dx, dy): (f64, f64)) {
        let vp = self.viewport();
        self.command(Command::Pan {
            dx: (2.0 * dx / vp.w.max(1) as f64) as f32,
            dy: (-2.0 * dy / vp.h.max(1) as f64) as f32,
        });
    }
    /// Wheel zoom that keeps the model point under the cursor where it is, so a small feature can be approached.
    pub(super) fn zoom_at(&mut self, lines: f32, at: (f64, f64)) {
        let vp = self.viewport();
        let (old_zoom, old_pan) = (self.view.zoom, self.view.pan);
        self.command(Command::Zoom(lines));
        let k = self.view.zoom / old_zoom;
        let c = [
            (2.0 * (at.0 - vp.x as f64) / vp.w.max(1) as f64 - 1.0) as f32,
            (1.0 - 2.0 * (at.1 - vp.y as f64) / vp.h.max(1) as f64) as f32,
        ];
        let pan = [
            c[0] - (c[0] - old_pan[0]) * k,
            c[1] - (c[1] - old_pan[1]) * k,
        ];
        self.command(Command::Pan {
            dx: pan[0] - self.view.pan[0],
            dy: pan[1] - self.view.pan[1],
        });
    }
}
