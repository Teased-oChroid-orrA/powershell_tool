//! The parts of the window: top bar, tool pill, feature tree, property manager, status bar.
use super::{
    theme::{tone_color, Palette},
    with_alpha, Gui,
};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::authoring::StepState,
    editor::authoring::{Card, Relevance},
    menu::Entry,
    menu::{Menu, Outcome},
    results::{result_card, results_tree, GuiInput},
    text::{colormap, fmt_value},
    ui::{visible_nodes, DocSidebar, DockItem, RibbonTab, TreeNode},
};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Margin, Order, Pos2, Rect, RichText,
    Sense, Stroke, Vec2,
};

pub(super) fn popup_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(8))
        .shadow(egui::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(90),
        })
}

fn bar_frame(p: &Palette, margin: Margin) -> egui::Frame {
    egui::Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.line))
        .inner_margin(margin)
}

pub(super) fn search_icon(ui: &egui::Ui, rect: Rect, name: &str, color: Color32) {
    let p = ui.painter();
    let c = rect.center();
    let s = Stroke::new(1.5, color);
    match name {
        "search" => {
            p.circle_stroke(c - Vec2::new(1.5, 1.5), 4.3, s);
            p.line_segment([c + Vec2::new(1.5, 1.5), c + Vec2::new(5.0, 5.0)], s);
        }
        "folder" => {
            let r = Rect::from_center_size(c + Vec2::new(0.0, 1.0), Vec2::new(14.0, 10.0));
            p.rect_stroke(r, CornerRadius::same(2), s, egui::StrokeKind::Inside);
            p.line_segment([r.left_top(), Pos2::new(r.left() + 4.0, r.top())], s);
            p.line_segment(
                [
                    Pos2::new(r.left() + 4.0, r.top()),
                    Pos2::new(r.left() + 6.0, r.top() + 2.0),
                ],
                s,
            );
        }
        "index" => {
            let r = Rect::from_center_size(c, Vec2::new(12.0, 12.0));
            p.rect_stroke(r, CornerRadius::same(4), s, egui::StrokeKind::Inside);
            for y in [-2.0, 2.0] {
                p.line_segment(
                    [
                        Pos2::new(r.left() + 1.0, c.y + y),
                        Pos2::new(r.right() - 1.0, c.y + y),
                    ],
                    Stroke::new(1.0, color),
                );
            }
        }
        "settings" => {
            for (y, x) in [(-4.0, -2.0), (0.0, 3.0), (4.0, -1.0)] {
                p.line_segment(
                    [Pos2::new(c.x - 6.0, c.y + y), Pos2::new(c.x + 6.0, c.y + y)],
                    s,
                );
                p.circle_filled(Pos2::new(c.x + x, c.y + y), 1.8, ui.visuals().panel_fill);
                p.circle_stroke(Pos2::new(c.x + x, c.y + y), 1.8, s);
            }
        }
        "report" => {
            let r = Rect::from_center_size(c, Vec2::new(10.0, 14.0));
            p.rect_stroke(r, CornerRadius::same(1), s, egui::StrokeKind::Inside);
            for y in [-2.0, 1.0, 4.0] {
                p.line_segment(
                    [
                        Pos2::new(r.left() + 2.0, c.y + y),
                        Pos2::new(r.right() - 2.0, c.y + y),
                    ],
                    s,
                );
            }
        }
        "copy" => {
            let back = Rect::from_center_size(c - Vec2::new(2.0, 2.0), Vec2::new(9.0, 11.0));
            let front = Rect::from_center_size(c + Vec2::new(2.0, 2.0), Vec2::new(9.0, 11.0));
            p.rect_stroke(
                back,
                CornerRadius::same(1),
                Stroke::new(1.0, color),
                egui::StrokeKind::Inside,
            );
            p.rect_stroke(front, CornerRadius::same(1), s, egui::StrokeKind::Inside);
        }
        "open" => {
            p.rect_stroke(
                Rect::from_center_size(c - Vec2::new(1.0, 1.0), Vec2::new(12.0, 12.0)),
                CornerRadius::same(1),
                s,
                egui::StrokeKind::Inside,
            );
            p.line_segment([c, c + Vec2::new(6.0, -6.0)], s);
            p.line_segment([c + Vec2::new(2.0, -6.0), c + Vec2::new(6.0, -6.0)], s);
            p.line_segment([c + Vec2::new(6.0, -6.0), c + Vec2::new(6.0, -2.0)], s);
        }
        _ => {}
    }
}

/// Dot colour of a tree node by its kind letter.
fn marker_color(
    p: &Palette,
    glyph: char,
    tone: crate::gpu_viewer::editor::session::Tone,
) -> Color32 {
    match glyph {
        '^' => p.good,
        '>' => p.warn,
        'R' | 'f' => p.accent,
        '#' => p.accent.gamma_multiply(0.7),
        'M' => p.accent,
        _ => tone_color(p, tone).gamma_multiply(0.8),
    }
}

impl Gui {
    pub(super) fn search_top_bar(
        &mut self,
        ui: &mut egui::Ui,
        menus: &[Menu],
        _sidebar: &DocSidebar,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        let bg = Color32::from_rgb(23, 42, 50);
        egui::Panel::top("bar")
            .exact_size(48.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(bg)
                    .inner_margin(Margin::symmetric(14, 6)),
            )
            .show(ui, |ui| {
                let compact = ui.available_width() < 420.0;
                let minimal = ui.available_width() < 300.0;
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let menu = ui.button("Menu");
                    self.targets.push(("Menu".into(), menu.rect));
                    if menu.clicked() {
                        self.search_menu_open = Some(
                            menus
                                .first()
                                .map_or("File", |m| m.title.as_str())
                                .to_string(),
                        );
                    }
                    if compact {
                        ui.label(
                            RichText::new("Search")
                                .strong()
                                .color(Color32::from_rgb(226, 238, 237))
                                .size(12.0),
                        );
                    } else {
                        ui.label(
                            RichText::new("Toolbench")
                                .color(Color32::from_rgb(154, 173, 170))
                                .size(12.0),
                        );
                        ui.label(RichText::new("/").color(Color32::from_rgb(93, 119, 126)));
                        ui.label(
                            RichText::new("Search")
                                .strong()
                                .color(Color32::from_rgb(226, 238, 237))
                                .size(12.0),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !minimal {
                            let avatar = ui.add_sized(
                                Vec2::splat(29.0),
                                egui::Button::new(
                                    RichText::new("GS")
                                        .strong()
                                        .size(10.0)
                                        .color(Color32::from_rgb(45, 82, 75)),
                                )
                                .fill(Color32::from_rgb(216, 232, 226))
                                .stroke(Stroke::NONE),
                            );
                            self.targets
                                .push(("GS Toolbench account".into(), avatar.rect));
                        }
                        let light = ui.ctx().theme() == egui::Theme::Dark;
                        let theme = ui.add(
                            egui::Button::new(if light { "Light" } else { "Dark" })
                                .small()
                                .frame(false),
                        );
                        self.targets.push(("Theme".into(), theme.rect));
                        if theme
                            .on_hover_text("Switch between light and dark theme")
                            .clicked()
                        {
                            ui.ctx().set_theme(if light {
                                egui::Theme::Light
                            } else {
                                egui::Theme::Dark
                            });
                        }
                        let help = ui.add(
                            egui::Button::new(
                                RichText::new(if compact { "Help" } else { "Help & shortcuts" })
                                    .color(Color32::from_rgb(196, 211, 211)),
                            )
                            .shortcut_text(if compact {
                                RichText::new("")
                            } else {
                                RichText::new("Ctrl K")
                                    .small()
                                    .color(Color32::from_rgb(141, 163, 170))
                            }),
                        );
                        self.targets.push(("Help and shortcuts".into(), help.rect));
                        if help.clicked() {
                            out.push(Outcome::Action(UiAction::OpenPalette));
                        }
                    });
                });
            });
        self.search_menu(ui.ctx(), menus, p, out, hint);
    }

    fn search_menu(
        &mut self,
        ctx: &egui::Context,
        menus: &[Menu],
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        let Some(open) = self.search_menu_open.as_deref() else {
            return;
        };
        let screen = ctx.content_rect();
        let scrim = egui::Area::new(Id::new("search-command-menu-scrim"))
            .order(Order::Middle)
            .fixed_pos(screen.min)
            .constrain(false)
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    Color32::from_rgba_unmultiplied(7, 15, 19, 163),
                );
                response.clicked()
            });
        if scrim.inner {
            self.search_menu_open = None;
            return;
        }
        let open = menus.iter().find(|m| m.title == open);
        let Some(menu) = open else {
            self.search_menu_open = None;
            return;
        };
        let mut close = false;
        egui::Area::new(Id::new("search-command-menu"))
            .order(Order::Foreground)
            .pivot(Align2::CENTER_CENTER)
            .fixed_pos(screen.center())
            .show(ctx, |ui| {
                popup_frame(p)
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(380.0);
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new("SEARCH WORKBENCH")
                                        .size(9.0)
                                        .strong()
                                        .color(p.accent),
                                );
                                ui.heading(&menu.title);
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let button = ui.button("×");
                                self.targets.push(("Close menu".into(), button.rect));
                                close |= button.clicked();
                            });
                        });
                        ui.horizontal(|ui| {
                            for item in menus {
                                let selected = item.title == menu.title;
                                let response = ui.selectable_label(selected, &item.title);
                                self.targets
                                    .push((format!("{} menu", item.title), response.rect));
                                if response.clicked() {
                                    self.search_menu_open = Some(item.title.clone());
                                }
                            }
                        });
                        ui.separator();
                        close |= self.items_ui(ui, &menu.items, p, out, hint);
                    });
            });
        if close {
            self.search_menu_open = None;
        }
    }

    pub(super) fn search_sidebar(
        &mut self,
        ui: &mut egui::Ui,
        sidebar: &DocSidebar,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        let nav = Color32::from_rgb(23, 42, 50);
        let nav_active = Color32::from_rgb(41, 73, 83);
        let ink = Color32::from_rgb(230, 240, 240);
        let muted = Color32::from_rgb(141, 163, 172);
        let teal = Color32::from_rgb(128, 219, 194);
        let line = Color32::from_rgb(53, 80, 91);
        let width = ui.available_width();
        let max_sidebar = (width - 360.0).clamp(120.0, 460.0);
        let min_sidebar = 120.0_f32.min(max_sidebar);
        let panel_id = Id::new("search-sidebar");
        let available = ui.available_rect_before_wrap();
        self.search_sidebar_width = self.search_sidebar_width.clamp(min_sidebar, max_sidebar);
        let sidebar_width = self.search_sidebar_width;
        egui::Panel::left(panel_id)
            .exact_size(sidebar_width)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(nav)
                    .inner_margin(Margin::symmetric(14, 20)),
            )
            .show(ui, |ui| {
                ui.take_available_width();
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        egui::Frame::new()
                            .fill(teal)
                            .corner_radius(CornerRadius::same(9))
                            .inner_margin(Margin::same(7))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new("G")
                                        .strong()
                                        .size(16.0)
                                        .color(Color32::from_rgb(18, 58, 60)),
                                );
                            });
                        ui.vertical(|ui| {
                            ui.label(RichText::new("GS Toolbench").strong().size(13.0).color(ink));
                            ui.label(
                                RichText::new("ENGINEERING WORKSPACE")
                                    .size(8.0)
                                    .color(muted),
                            );
                        });
                    });
                    ui.add_space(28.0);
                    ui.label(
                        RichText::new("WORKSPACE")
                            .strong()
                            .size(9.0)
                            .color(Color32::from_rgb(119, 144, 154)),
                    );
                    ui.add_space(5.0);
                    let (search_rect, search_response) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), 36.0),
                        egui::Sense::click(),
                    );
                    ui.painter().rect(
                        search_rect,
                        CornerRadius::same(9),
                        if search_response.hovered() {
                            nav_active.gamma_multiply(1.15)
                        } else {
                            nav_active
                        },
                        Stroke::NONE,
                        egui::StrokeKind::Inside,
                    );
                    search_icon(
                        ui,
                        Rect::from_center_size(
                            Pos2::new(search_rect.left() + 19.0, search_rect.center().y),
                            Vec2::splat(16.0),
                        ),
                        "search",
                        Color32::from_rgb(140, 224, 199),
                    );
                    ui.painter().text(
                        Pos2::new(search_rect.left() + 35.0, search_rect.center().y),
                        Align2::LEFT_CENTER,
                        "Search",
                        FontId::proportional(12.0),
                        ink,
                    );
                    let search = search_response;
                    self.targets.push(("Search workspace".into(), search_rect));
                    if search.clicked() {
                        self.adapt.tree_used();
                    }
                    ui.horizontal(|ui| {
                        let (icon, _) =
                            ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
                        search_icon(ui, icon, "index", Color32::from_rgb(185, 201, 205));
                        ui.label(
                            RichText::new("Fast index")
                                .size(12.0)
                                .color(Color32::from_rgb(185, 201, 205)),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            egui::Frame::new()
                                .stroke(Stroke::new(1.0, line))
                                .corner_radius(CornerRadius::same(8))
                                .inner_margin(Margin::symmetric(7, 2))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(&sidebar.index_state)
                                            .size(9.0)
                                            .color(Color32::from_rgb(184, 201, 203)),
                                    );
                                });
                        });
                    });
                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("RECENT SEARCHES")
                                .strong()
                                .size(9.0)
                                .color(Color32::from_rgb(119, 144, 154)),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if let Some(action) = sidebar.clear_recents {
                                let clear = ui.add(
                                    egui::Button::new(
                                        RichText::new("Clear all").size(9.0).color(muted),
                                    )
                                    .frame(false),
                                );
                                self.targets
                                    .push(("Clear recent searches".into(), clear.rect));
                                if clear.clicked() {
                                    out.push(Outcome::Action(action));
                                }
                            }
                        });
                    });
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .id_salt("search-sidebar-recents")
                        .auto_shrink([false, true])
                        .max_height((ui.available_height() - 190.0).max(0.0))
                        .show(ui, |ui| {
                            for (index, recent) in sidebar.recent_searches.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    let (dot, _) = ui.allocate_exact_size(
                                        Vec2::splat(8.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().circle_filled(
                                        dot.center(),
                                        3.0,
                                        if index == 0 {
                                            teal
                                        } else {
                                            Color32::from_rgb(129, 164, 220)
                                        },
                                    );
                                    let label_width = recent_label_width(
                                        (ui.available_width() - 14.0).max(0.0),
                                        ui.spacing().item_spacing.x,
                                    );
                                    let label = ui.add_sized(
                                        Vec2::new(label_width, 32.0),
                                        egui::Label::new(
                                            RichText::new(&recent.label)
                                                .size(11.0)
                                                .color(Color32::from_rgb(185, 201, 205)),
                                        )
                                        .truncate()
                                        .sense(egui::Sense::click()),
                                    );
                                    let remove = ui.add_sized(
                                        Vec2::splat(22.0),
                                        egui::Button::new(
                                            RichText::new("×")
                                                .size(15.0)
                                                .color(Color32::TRANSPARENT),
                                        )
                                        .fill(Color32::TRANSPARENT)
                                        .stroke(Stroke::NONE),
                                    );
                                    let revealed =
                                        label.hovered() || remove.hovered() || remove.has_focus();
                                    if revealed {
                                        ui.painter().text(
                                            remove.rect.center(),
                                            Align2::CENTER_CENTER,
                                            "×",
                                            FontId::proportional(15.0),
                                            if remove.hovered() {
                                                Color32::from_rgb(212, 135, 127)
                                            } else {
                                                muted
                                            },
                                        );
                                    }
                                    self.targets.push((recent.label.clone(), label.rect));
                                    self.targets.push((
                                        format!("Remove recent search {}", index + 1),
                                        remove.rect,
                                    ));
                                    if label.on_hover_text(&recent.label).clicked() {
                                        out.push(Outcome::Action(recent.action));
                                    }
                                    if remove.on_hover_text("Remove this search").clicked() {
                                        out.push(Outcome::Action(recent.remove));
                                    }
                                });
                            }
                            if sidebar.recent_searches.is_empty() {
                                ui.label(
                                    RichText::new("Saved searches will appear here.")
                                        .small()
                                        .color(muted),
                                );
                            }
                        });
                    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                        ui.label(
                            RichText::new("Offline-ready · Local files stay local")
                                .size(9.0)
                                .color(Color32::from_rgb(114, 137, 146)),
                        );
                        ui.add_space(12.0);
                        egui::Frame::new()
                            .fill(Color32::from_rgb(29, 53, 65))
                            .stroke(Stroke::new(1.0, line))
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(Margin::same(11))
                            .show(ui, |ui| {
                                let card_width = (ui.available_width() - 14.0).max(0.0);
                                ui.set_max_width(card_width);
                                ui.set_width(card_width);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("LOCAL INDEX")
                                            .strong()
                                            .size(9.0)
                                            .color(Color32::from_rgb(197, 213, 216)),
                                    );
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(RichText::new("●").color(
                                            if sidebar.index_state == "Ready" {
                                                Color32::from_rgb(102, 208, 169)
                                            } else {
                                                muted
                                            },
                                        ))
                                    });
                                });
                                ui.add_space(7.0);
                                ui.label(
                                    RichText::new(&sidebar.index_name)
                                        .strong()
                                        .size(11.5)
                                        .color(ink),
                                );
                                ui.label(
                                    RichText::new(&sidebar.index_detail).size(9.0).color(muted),
                                );
                                if let Some(action) = &sidebar.index_action {
                                    ui.add_space(6.0);
                                    let button = ui.add(
                                        egui::Button::new(
                                            RichText::new(&action.label).size(9.5).color(teal),
                                        )
                                        .small(),
                                    );
                                    self.targets.push((action.label.clone(), button.rect));
                                    if button.clicked() {
                                        out.push(Outcome::Action(action.action));
                                    }
                                }
                            });
                    });
                });
            });
        let divider_x = available.left() + sidebar_width;
        let resize_handle = Rect::from_min_max(
            Pos2::new(divider_x - 12.0, available.top() + 48.0),
            Pos2::new(divider_x + 12.0, available.bottom() - 26.0),
        );
        let resize = ui.interact(resize_handle, panel_id.with("resize"), egui::Sense::drag());
        let highlighted = resize.hovered() || resize.dragged();
        if highlighted {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        ui.painter().line_segment(
            [
                Pos2::new(divider_x, available.top()),
                Pos2::new(divider_x, available.bottom()),
            ],
            Stroke::new(
                if highlighted { 2.0 } else { 1.0 },
                if highlighted { teal } else { line },
            ),
        );
        if resize.dragged() || resize.drag_stopped() {
            if let Some(pointer) = resize.interact_pointer_pos() {
                self.search_sidebar_width =
                    (pointer.x - available.left()).clamp(min_sidebar, max_sidebar);
                ui.ctx().request_repaint();
            }
        }
        let _ = (p, hint);
    }

    pub(super) fn search_status_bar(&mut self, ui: &mut egui::Ui, input: &GuiInput, _p: &Palette) {
        egui::Panel::bottom("status")
            .exact_size(26.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(32, 45, 51))
                    .inner_margin(Margin::symmetric(12, 4)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let compact = ui.available_width() < 560.0;
                    let status = input
                        .editor
                        .map(|e| e.status())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| {
                            "Local search · File contents are processed on this device".into()
                        });
                    ui.add(
                        egui::Label::new(
                            RichText::new(status)
                                .size(10.0)
                                .color(Color32::from_rgb(154, 173, 170)),
                        )
                        .truncate(),
                    );
                    if !compact {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(
                                    "↑ ↓ move through results   Enter open file   Ctrl+C copy path",
                                )
                                .size(9.0)
                                .color(Color32::from_rgb(114, 137, 146)),
                            );
                        });
                    }
                });
            });
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn top_bar(
        &mut self,
        ui: &mut egui::Ui,
        input: &GuiInput,
        menus: &[Menu],
        tabs: &[RibbonTab],
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        egui::Panel::top("bar")
            .exact_size(40.0)
            .show_separator_line(false)
            .frame(bar_frame(p, Margin::symmetric(10, 5)))
            .show(ui, |ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.menu_button("Menu", |ui| {
                        for m in menus {
                            ui.menu_button(&m.title, |ui| {
                                ui.set_min_width(230.0);
                                self.items_ui(ui, &m.items, p, out, hint);
                            });
                        }
                    });
                    if let Some(ed) = input.editor {
                        ui.label(RichText::new(ed.title()).strong());
                    }
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(2.0);
                    let mut chosen = None;
                    // A page bench has no tool pill: its steps would switch nothing.
                    let steps = if input.editor.is_some_and(|e| e.has_document()) {
                        &[][..]
                    } else {
                        tabs
                    };
                    for (i, tab) in steps.iter().enumerate() {
                        let state = input.editor.and_then(|e| e.step_state(&tab.title));
                        let nudged = self.nudge.as_deref() == Some(tab.title.as_str());
                        if self
                            .step_chip(ui, &tab.title, state, i == self.tab, nudged, p)
                            .clicked()
                        {
                            chosen = Some(i);
                        }
                    }
                    if let Some(i) = chosen {
                        self.select_tab(i, tabs);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let dark = ui.ctx().theme() == egui::Theme::Dark;
                        let r =
                            ui.add(egui::Button::new(if dark { "Light" } else { "Dark" }).small());
                        self.targets.push(("Theme".into(), r.rect));
                        if r.on_hover_text("Switch between the light and the dark theme")
                            .clicked()
                        {
                            ui.ctx().set_theme(if dark {
                                egui::Theme::Light
                            } else {
                                egui::Theme::Dark
                            });
                        }
                        let r = ui.add(
                            egui::Button::new("Search commands")
                                .shortcut_text(RichText::new("Ctrl K").small().color(p.dim)),
                        );
                        self.targets.push(("Search commands".into(), r.rect));
                        if r.clicked() {
                            out.push(Outcome::Action(UiAction::OpenPalette));
                        }
                    });
                });
            });
    }

    /// One step of the thin strip: a status dot (done, or still to do) and the step's name.
    fn step_chip(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        state: Option<StepState>,
        selected: bool,
        nudged: bool,
        p: &Palette,
    ) -> egui::Response {
        let galley = ui.painter().layout_no_wrap(
            label.to_string(),
            FontId::proportional(13.0),
            if selected { p.text } else { p.dim },
        );
        let w = galley.size().x + if state.is_some() { 32.0 } else { 20.0 };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::click());
        self.targets.push((format!("{label} tab"), rect));
        let hovered = resp.hovered();
        if selected {
            ui.painter()
                .rect_filled(rect, 13.0, with_alpha(p.accent, 40));
        } else if hovered {
            ui.painter().rect_filled(rect, 13.0, p.raised);
        }
        if nudged {
            // The step this person usually takes next: a ring, not a jump.
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                13.0,
                Stroke::new(1.5, p.accent),
                egui::StrokeKind::Inside,
            );
        }
        let mut x = rect.left() + 10.0;
        if let Some(s) = state {
            let c = Pos2::new(x + 4.0, rect.center().y);
            match s {
                StepState::Done => {
                    ui.painter().circle_filled(c, 4.0, p.good);
                }
                StepState::Todo => {
                    ui.painter().circle_stroke(c, 3.5, Stroke::new(1.3, p.dim));
                }
            }
            x += 14.0;
        }
        ui.painter().galley(
            Pos2::new(x, rect.center().y - galley.size().y / 2.0),
            galley,
            p.text,
        );
        resp
    }

    /// The tools of the selected step, floating over the top of the model. Inside each group the most-used tools come
    /// first once that is learned (`adapt`); a long group keeps its most-used ones in view and the rest under More.
    pub(super) fn tool_pill(
        &mut self,
        ctx: &egui::Context,
        tabs: &[RibbonTab],
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        let Some(tab) = self.selected_tab(tabs).cloned() else {
            return;
        };
        let Some(vp) = self.viewport else { return };
        // With the tree folded to its chip the pill starts to the right of the chip instead of running under it.
        let (pivot, at_x, width) = match self.chip_rect {
            Some(chip) => {
                let left = chip.right() + 10.0;
                (
                    Align2::LEFT_TOP,
                    left,
                    (vp.right() - 12.0 - left - 24.0).max(160.0),
                )
            }
            None => (
                Align2::CENTER_TOP,
                vp.center().x,
                (vp.width() - 40.0).max(160.0),
            ),
        };
        let shown = egui::Area::new(Id::new("tool-pill"))
            .fade_in(false)
            .order(Order::Middle)
            .pivot(pivot)
            .fixed_pos(Pos2::new(at_x, vp.top() + 12.0))
            .show(ctx, |ui| {
                popup_frame(p)
                    .fill(p.raised)
                    .inner_margin(Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.set_max_width(width);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                            for (g, group) in tab.groups.iter().enumerate() {
                                if g > 0 {
                                    ui.add_space(6.0);
                                    ui.separator();
                                    ui.add_space(6.0);
                                }
                                ui.label(RichText::new(&group.title).small().color(p.dim));
                                let key = format!("tool/{}/{}", tab.title, group.title);
                                let labels: Vec<String> =
                                    group.buttons.iter().map(|e| e.label.clone()).collect();
                                self.adapt.offer(
                                    &key,
                                    &labels.iter().map(String::as_str).collect::<Vec<_>>(),
                                );
                                let ordered: Vec<&Entry> = self
                                    .adapt
                                    .order(&key, &labels)
                                    .iter()
                                    .filter_map(|l| group.buttons.iter().find(|e| &e.label == l))
                                    .collect();
                                let learned = self.adapt.enabled
                                    && labels.iter().any(|l| self.adapt.score(&key, l).is_some());
                                let shown = if learned && ordered.len() > MAX_VISIBLE_TOOLS {
                                    MAX_VISIBLE_TOOLS
                                } else {
                                    ordered.len()
                                };
                                for e in &ordered[..shown] {
                                    if self.entry_button(ui, e, p, out, hint, false) {
                                        self.adapt.used(&key, &e.label);
                                    }
                                }
                                if shown < ordered.len() {
                                    let rest = &ordered[shown..];
                                    ui.menu_button(
                                        RichText::new(format!("More ({})", rest.len()))
                                            .color(p.dim),
                                        |ui| {
                                            for e in rest {
                                                if self.entry_button(ui, e, p, out, hint, true) {
                                                    self.adapt.used(&key, &e.label);
                                                }
                                            }
                                        },
                                    );
                                }
                            }
                        });
                    });
            });
        self.pill_rect = Some(shown.response.rect);
    }

    /// The tree folded to a chip over the model; one click opens it.
    pub(super) fn tree_chip(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(vp) = self.viewport else { return };
        let shown = egui::Area::new(Id::new("tree-chip"))
            .fade_in(false)
            .order(Order::Middle)
            .fixed_pos(Pos2::new(vp.left() + 12.0, vp.top() + 12.0))
            .show(ctx, |ui| {
                popup_frame(p)
                    .fill(p.raised)
                    .inner_margin(Margin::symmetric(4, 3))
                    .show(ui, |ui| {
                        let r = ui.add(egui::Button::new("Feature tree").small());
                        self.targets.push(("Feature tree".into(), r.rect));
                        if r.on_hover_text("Open the feature tree").clicked() {
                            self.tree_user = Some(true);
                            self.adapt.tree_used();
                        }
                    });
            });
        self.chip_rect = Some(shown.response.rect);
    }

    /// An empty model shows how to begin, centred over the viewport.
    pub(super) fn start_card(
        &mut self,
        ctx: &egui::Context,
        input: &GuiInput,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        let options = input.editor.map(|e| e.start_options()).unwrap_or_default();
        let (Some(vp), false) = (self.viewport, options.is_empty()) else {
            return;
        };
        egui::Area::new(Id::new("start-card"))
            .fade_in(false)
            .order(Order::Middle)
            .pivot(Align2::CENTER_CENTER)
            .fixed_pos(vp.center())
            .show(ctx, |ui| {
                popup_frame(p)
                    .fill(p.panel)
                    .inner_margin(Margin::symmetric(22, 18))
                    .show(ui, |ui| {
                        ui.set_width(300.0);
                        ui.label(RichText::new("Start a part").size(18.0).strong());
                        ui.label(
                            RichText::new(
                                "Draw a sketch, begin from a template, or open a saved model.",
                            )
                            .color(p.dim),
                        );
                        ui.add_space(8.0);
                        for (k, e) in options.iter().enumerate() {
                            if k == 1 {
                                ui.add_space(4.0);
                                ui.label(RichText::new("TEMPLATES").small().strong().color(p.dim));
                            }
                            if k + 1 == options.len() && k > 0 {
                                ui.add_space(6.0);
                            }
                            self.entry_button(ui, e, p, out, hint, true);
                        }
                    });
            });
    }

    pub(super) fn status_bar(
        &mut self,
        ui: &mut egui::Ui,
        input: &GuiInput,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        let _ = hint;
        egui::Panel::bottom("status").exact_size(26.0).show_separator_line(false).frame(bar_frame(p, Margin::symmetric(12, 4))).show(ui, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let running = input.editor.is_some_and(|e| e.solving_for().is_some());
                let status = input.editor.map(|e| e.status()).filter(|s| !s.is_empty());
                let (text, alert) = match (input.error, running, &self.hint, status) {
                    (Some(e), _, _, _) => (e.to_string(), true),
                    (None, true, _, Some(s)) => (s, false),
                    (None, _, Some(h), _) => (h.clone(), false),
                    (None, _, None, Some(s)) => (s, false),
                    _ => ("Ready".to_string(), false),
                };
                // After a failed solve the best fix is one click away.
                if let Some((label, action)) = input.editor.and_then(|e| e.status_fix()) {
                    let r = ui.add(egui::Button::new(RichText::new(label).color(p.accent)).small());
                    self.targets.push(("Apply the suggested fix".into(), r.rect));
                    if r.on_hover_text("Apply the best fix for this failure (one undoable edit); the Solve failed card lists the others").clicked() {
                        out.push(Outcome::Action(action));
                    }
                }
                ui.add(egui::Label::new(RichText::new(text).color(if alert { p.warn } else { p.text })).truncate());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let scene = input.scene();
                    let mut right = format!("result {}/{} | {}", input.selected + 1, input.scenes.len(), input.view.field_name(scene));
                    if let Some(e) = input.editor.filter(|e| e.has_document()) {
                        right = e.solving_for().map_or_else(|| "Ready".to_string(), |t| format!("Working... {:.1}s", t.as_secs_f32()));
                    } else if let Some(e) = input.editor {
                        right = format!("{} | {right}", if let Some(elapsed) = e.solving_for() { format!("Working... {:.1}s", elapsed.as_secs_f32()) } else if e.dirty() { "Modified".into() } else { "Saved".into() });
                    }
                    ui.label(RichText::new(right).small().color(p.dim));
                });
            });
        });
    }

    pub(super) fn tree_panel(
        &mut self,
        ui: &mut egui::Ui,
        input: &GuiInput,
        p: &Palette,
        out: &mut Vec<Outcome>,
    ) {
        let mut nodes = input.editor.map(|e| e.tree()).unwrap_or_default();
        nodes.extend(results_tree(input));
        let page = input.editor.is_some_and(|editor| editor.has_document());
        egui::Panel::left("tree")
            .default_size(if page { 224.0 } else { 240.0 })
            .size_range(170.0..=460.0)
            .frame(bar_frame(p, Margin::symmetric(8, 8)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    section_title(ui, if page { "Workspace" } else { "Feature tree" }, p);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let r = ui.add(egui::Button::new("Fold").small().frame(false));
                        self.targets.push(("Fold the feature tree".into(), r.rect));
                        if r.on_hover_text("Fold the tree to a chip (it comes back with one click)")
                            .clicked()
                        {
                            self.tree_user = Some(false);
                        }
                    });
                });
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let shown: Vec<TreeNode> = visible_nodes(&nodes, &self.collapsed)
                            .into_iter()
                            .cloned()
                            .collect();
                        for n in &shown {
                            self.tree_row(ui, n, p, out);
                        }
                    });
            });
    }

    fn tree_row(&mut self, ui: &mut egui::Ui, n: &TreeNode, p: &Palette, out: &mut Vec<Outcome>) {
        let color = if n.selected {
            p.text
        } else {
            tone_color(p, n.tone)
        };
        let indent = 6.0 + f32::from(n.depth) * 14.0 + 16.0;
        // A long label wraps; the row grows to fit it instead of running into its neighbours.
        let galley = ui.painter().layout(
            n.label.clone(),
            FontId::proportional(13.0),
            color,
            (ui.available_width() - indent - 6.0).max(40.0),
        );
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), (galley.size().y + 8.0).max(24.0)),
            Sense::click(),
        );
        self.targets.push((n.label.clone(), rect));
        let painter = ui.painter();
        if n.selected {
            painter.rect_filled(rect, 5.0, with_alpha(p.accent, 46));
        } else if resp.hovered() && (n.action.is_some() || n.collapsible) {
            painter.rect_filled(rect, 5.0, p.raised);
        }
        let x0 = rect.left() + 6.0 + f32::from(n.depth) * 14.0;
        let mid = rect.center().y;
        let collapsed = self.collapsed.contains(&n.id);
        let mut text_x = x0;
        if n.collapsible {
            // A triangle: right when folded, down when open.
            let c = Pos2::new(x0 + 5.0, mid);
            let pts = if collapsed {
                vec![
                    c + Vec2::new(-2.5, -4.0),
                    c + Vec2::new(3.5, 0.0),
                    c + Vec2::new(-2.5, 4.0),
                ]
            } else {
                vec![
                    c + Vec2::new(-4.0, -2.5),
                    c + Vec2::new(4.0, -2.5),
                    c + Vec2::new(0.0, 3.5),
                ]
            };
            painter.add(egui::Shape::convex_polygon(pts, p.dim, Stroke::NONE));
            text_x += 16.0;
        } else {
            painter.circle_filled(
                Pos2::new(x0 + 5.0, mid),
                3.0,
                marker_color(p, n.glyph, n.tone),
            );
            text_x += 16.0;
        }
        painter.galley(
            Pos2::new(text_x, mid - galley.size().y / 2.0),
            galley,
            color,
        );
        if resp.clicked() || resp.secondary_clicked() {
            self.adapt.tree_used();
        }
        if resp.clicked() {
            if n.collapsible && n.action.is_none() {
                if !self.collapsed.remove(&n.id) {
                    self.collapsed.insert(n.id);
                }
            } else if let Some(a) = n.action {
                out.push(Outcome::Action(a));
            }
        }
        // A right click selects the row and opens the menu of its entity (next frame, from the selected state).
        if resp.secondary_clicked() {
            if let Some(a) = n.action {
                out.push(Outcome::Action(a));
            }
            if let Some(at) = resp.interact_pointer_pos() {
                self.pending_tree_menu = Some((at, n.id));
            }
        }
    }

    /// Decide what the inspector shows: the editor's cards for this step, relevance first and the learned order inside
    /// a level. `None` when there is nothing to show or the person closed it for this selection. Called before the
    /// viewport is fixed because the cards reserve room on the right (the model is never drawn under them).
    pub(super) fn inspector_plan(
        &mut self,
        input: &GuiInput,
        tabs: &[RibbonTab],
    ) -> Option<InspectorPlan> {
        let step = tabs
            .get(self.tab)
            .map_or("", |t| t.title.as_str())
            .to_string();
        let mut cards: Vec<Card> = input.editor.map(|e| e.inspector()).unwrap_or_default();
        if input.editor.is_none() || !input.scene().fields.is_empty() {
            cards.push(result_card(input));
        }
        cards.retain(|c| {
            (c.steps.is_empty() || c.steps.iter().any(|s| *s == step)) && !c.items.is_empty()
        });
        if cards.is_empty() {
            return None;
        }
        let ctx_key = format!("card/{step}");
        let ids: Vec<&str> = cards.iter().map(|c| c.id.as_str()).collect();
        self.adapt.offer(&ctx_key, &ids);
        let mut ordered: Vec<Card> = Vec::new();
        for level in [Relevance::Primary, Relevance::Context, Relevance::Info] {
            let ids: Vec<String> = cards
                .iter()
                .filter(|c| c.relevance == level)
                .map(|c| c.id.clone())
                .collect();
            ordered.extend(
                self.adapt
                    .order(&ctx_key, &ids)
                    .into_iter()
                    .filter_map(|id| cards.iter().find(|c| c.id == id).cloned()),
            );
        }
        let signature = ordered
            .iter()
            .map(|c| format!("{}:{}", c.id, c.title))
            .collect::<Vec<_>>()
            .join("|");
        if !self.inspector_pinned && self.inspector_dismissed.as_deref() == Some(signature.as_str())
        {
            return None;
        }
        Some(InspectorPlan {
            cards: ordered,
            ctx_key,
            signature,
        })
    }

    /// The inspector: floating cards at the right edge, in the room `inspector_plan` reserved.
    pub(super) fn inspector(
        &mut self,
        ctx: &egui::Context,
        plan: &InspectorPlan,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        let Some(vp) = self.viewport else { return };
        let top = vp.top() + 12.0;
        let max_h = (vp.bottom() - top - 12.0).max(120.0);
        egui::Area::new(Id::new("inspector"))
            .fade_in(false)
            .order(Order::Middle)
            .anchor(Align2::RIGHT_TOP, Vec2::new(-12.0, top))
            .show(ctx, |ui| {
                popup_frame(p)
                    .fill(p.panel)
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(INSPECTOR_WIDTH);
                        // An area's content starts 400 points tall (egui's default area size); the scroll area needs the room.
                        ui.set_max_height(max_h);
                        egui::ScrollArea::vertical()
                            .max_height(max_h)
                            .auto_shrink([true, true])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                for (k, card) in plan.cards.iter().enumerate() {
                                    if k > 0 {
                                        ui.add_space(6.0);
                                    }
                                    self.card_ui(
                                        ui,
                                        card,
                                        &plan.ctx_key,
                                        k == 0,
                                        &plan.signature,
                                        p,
                                        out,
                                        hint,
                                    );
                                }
                            });
                    });
            });
    }

    #[allow(clippy::too_many_arguments)]
    fn card_ui(
        &mut self,
        ui: &mut egui::Ui,
        card: &Card,
        ctx_key: &str,
        first: bool,
        signature: &str,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        let folded_by_habit = self.adapt.idle(ctx_key, &card.id);
        let open = *self.card_open.get(&card.id).unwrap_or(&!folded_by_habit);
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
        self.targets.push((format!("{} card", card.title), rect));
        let painter = ui.painter();
        if resp.hovered() {
            painter.rect_filled(rect, 5.0, p.raised);
        }
        let mid = rect.center().y;
        let c = Pos2::new(rect.left() + 10.0, mid);
        let tri = if open {
            vec![
                c + Vec2::new(-4.0, -2.5),
                c + Vec2::new(4.0, -2.5),
                c + Vec2::new(0.0, 3.5),
            ]
        } else {
            vec![
                c + Vec2::new(-2.5, -4.0),
                c + Vec2::new(3.5, 0.0),
                c + Vec2::new(-2.5, 4.0),
            ]
        };
        painter.add(egui::Shape::convex_polygon(tri, p.dim, Stroke::NONE));
        let title = painter.layout_no_wrap(
            card.title.to_uppercase(),
            FontId::proportional(11.0),
            if card.relevance == Relevance::Primary {
                p.accent
            } else {
                p.dim
            },
        );
        painter.galley(
            Pos2::new(rect.left() + 22.0, mid - title.size().y / 2.0),
            title,
            p.dim,
        );
        if resp.clicked() {
            self.card_open.insert(card.id.clone(), !open);
            self.adapt.used(ctx_key, &card.id);
        }
        if first {
            // Close and pin live on the first card's header.
            let pin = painter_text_button(
                ui,
                rect,
                p,
                if self.inspector_pinned {
                    "Pinned"
                } else {
                    "Pin"
                },
                52.0,
                30.0,
            );
            let close = painter_text_button(ui, rect, p, "Close", 0.0, 34.0);
            self.targets.push(("Pin the inspector".into(), pin.rect));
            self.targets
                .push(("Close the inspector".into(), close.rect));
            if pin
                .on_hover_text("Keep the inspector open when the selection changes")
                .clicked()
            {
                self.inspector_pinned = !self.inspector_pinned;
            }
            if close
                .on_hover_text("Close the inspector; it returns for the next selection")
                .clicked()
            {
                self.inspector_dismissed = Some(signature.to_string());
            }
        }
        if open {
            for item in &card.items {
                let before = out.len();
                self.dock_item(ui, item, p, out, hint);
                if out.len() > before {
                    self.adapt.used(ctx_key, &card.id);
                }
            }
        }
    }

    pub(super) fn dock_item(
        &mut self,
        ui: &mut egui::Ui,
        item: &DockItem,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        let _ = hint;
        match item {
            DockItem::Heading(h) => {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(h.to_uppercase())
                        .small()
                        .strong()
                        .color(p.dim),
                );
                ui.add(egui::Separator::default().spacing(2.0));
            }
            DockItem::Gap => {
                ui.add_space(4.0);
            }
            DockItem::Bracket { lower, upper } => {
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 12.0), Sense::hover());
                ui.painter().rect_filled(rect, 4.0, with_alpha(p.warn, 55));
                let verified = (*lower / *upper).clamp(0.0, 1.0) as f32;
                let fill = Rect::from_min_max(
                    rect.min,
                    Pos2::new(rect.left() + rect.width() * verified, rect.bottom()),
                );
                ui.painter().rect_filled(fill, 4.0, p.good);
            }
            DockItem::Line {
                text,
                tone,
                action,
                focused,
                hover,
            } => {
                let color = tone_color(p, *tone);
                match action {
                    Some(a) => {
                        let r = ui.add(
                            egui::Button::selectable(*focused, RichText::new(text).color(color))
                                .wrap()
                                .frame_when_inactive(false)
                                .min_size(Vec2::new(ui.available_width(), 0.0)),
                        );
                        self.targets.push((text.clone(), r.rect));
                        if r.hovered() && hover.is_some() {
                            self.hover_now = *hover;
                        }
                        if r.clicked() {
                            out.push(Outcome::Action(*a));
                        }
                    }
                    None => {
                        ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
                    }
                }
            }
            DockItem::Field {
                label,
                value,
                tone,
                action,
                focused,
            } => {
                let editing = *tone == crate::gpu_viewer::editor::session::Tone::Warn;
                let sense = if action.is_some() {
                    Sense::click()
                } else {
                    Sense::hover()
                };
                let width = ui.available_width();
                let value_galley =
                    ui.painter()
                        .layout_no_wrap(value.clone(), FontId::monospace(12.0), p.text);
                let vw = value_galley.size().x.min(width * 0.6);
                let label_galley = ui.painter().layout(
                    label.clone(),
                    FontId::proportional(13.0),
                    p.dim,
                    (width - vw - 22.0).max(20.0),
                );
                let (rect, resp) = ui.allocate_exact_size(
                    Vec2::new(width, (label_galley.size().y + 8.0).max(24.0)),
                    sense,
                );
                self.targets.push((label.clone(), rect));
                let painter = ui.painter();
                if *focused {
                    painter.rect_filled(rect, 5.0, with_alpha(p.accent, 40));
                } else if resp.hovered() && action.is_some() {
                    painter.rect_filled(rect, 5.0, p.raised);
                }
                let mid = rect.center().y;
                let vcol = if editing {
                    p.warn
                } else if *tone == crate::gpu_viewer::editor::session::Tone::On {
                    p.good
                } else {
                    p.text
                };
                let value_galley =
                    painter.layout_no_wrap(value.clone(), FontId::monospace(12.0), vcol);
                painter.galley(
                    Pos2::new(rect.left() + 6.0, mid - label_galley.size().y / 2.0),
                    label_galley,
                    p.dim,
                );
                if editing {
                    let box_rect = Rect::from_min_max(
                        Pos2::new(rect.right() - vw - 14.0, rect.top() + 2.0),
                        Pos2::new(rect.right() - 2.0, rect.bottom() - 2.0),
                    );
                    painter.rect_stroke(
                        box_rect,
                        4.0,
                        Stroke::new(1.0, p.warn),
                        egui::StrokeKind::Inside,
                    );
                }
                painter.galley(
                    Pos2::new(rect.right() - vw - 8.0, mid - value_galley.size().y / 2.0),
                    value_galley,
                    vcol,
                );
                if let (Some(a), true) = (action, resp.clicked()) {
                    out.push(Outcome::Action(*a));
                }
            }
            DockItem::Legend { lo, hi } => {
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 14.0), Sense::hover());
                let mut mesh = egui::Mesh::default();
                const STEPS: usize = 32;
                for k in 0..=STEPS {
                    let t = k as f32 / STEPS as f32;
                    let [r, g, b] = colormap(t);
                    let x = rect.left() + rect.width() * t;
                    mesh.colored_vertex(Pos2::new(x, rect.top()), Color32::from_rgb(r, g, b));
                    mesh.colored_vertex(Pos2::new(x, rect.bottom()), Color32::from_rgb(r, g, b));
                    if k > 0 {
                        let i = (2 * k) as u32;
                        mesh.add_triangle(i - 2, i - 1, i);
                        mesh.add_triangle(i - 1, i + 1, i);
                    }
                }
                ui.painter().add(egui::Shape::mesh(mesh));
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(fmt_value(*lo))
                            .small()
                            .monospace()
                            .color(p.dim),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(fmt_value(*hi))
                                .small()
                                .monospace()
                                .color(p.dim),
                        );
                    });
                });
            }
        }
    }
}

fn recent_label_width(available_width: f32, item_spacing: f32) -> f32 {
    (available_width - 8.0 - 22.0 - 2.0 * item_spacing).max(0.0)
}

#[cfg(test)]
mod sidebar_layout_tests {
    use super::recent_label_width;

    #[test]
    fn recent_search_row_fits_narrow_sidebar() {
        let available = 142.0;
        let spacing = 8.0;
        let label = recent_label_width(available, spacing);
        assert!(label + 8.0 + 22.0 + 2.0 * spacing <= available);
    }
}

fn section_title(ui: &mut egui::Ui, title: &str, p: &Palette) {
    ui.label(
        RichText::new(title.to_uppercase())
            .small()
            .strong()
            .color(p.accent),
    );
    ui.add_space(2.0);
}

/// A small text button laid over a card header, `from_right` points in from the header's right edge.
fn painter_text_button(
    ui: &mut egui::Ui,
    header: Rect,
    p: &Palette,
    label: &str,
    from_right: f32,
    width: f32,
) -> egui::Response {
    let rect = Rect::from_min_size(
        Pos2::new(
            header.right() - from_right - width - 2.0,
            header.top() + 2.0,
        ),
        Vec2::new(width, header.height() - 4.0),
    );
    let resp = ui.interact(rect, Id::new(("header-button", label)), Sense::click());
    let colour = if resp.hovered() { p.text } else { p.dim };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(11.0),
        colour,
    );
    resp
}

/// A group longer than this keeps its most-used tools in view and puts the rest under "More" (only once habits are learned).
const MAX_VISIBLE_TOOLS: usize = 8;

/// Width of the inspector's cards.
pub(super) const INSPECTOR_WIDTH: f32 = 330.0;

/// What `Gui::inspector_plan` decided to show.
pub(super) struct InspectorPlan {
    cards: Vec<Card>,
    ctx_key: String,
    signature: String,
}
