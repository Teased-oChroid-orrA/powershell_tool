//! The two edges a page can add around its cards (`Document::rail`, `Document::basket`): a left rail of choices that
//! narrow the table (Material Lookup's Refine) and a bottom tray of the materials set aside for comparison. Both
//! only draw data the session built; a click leaves as an `Outcome`.
use super::super::{theme::Palette, with_alpha, Gui};
use crate::gpu_viewer::{
    menu::Outcome,
    ui::{DocBasket, DocNarrow, Document},
};
use egui::{
    Align, Align2, CornerRadius, FontId, Id, Layout, Pos2, Rect, Sense, Stroke, UiBuilder, Vec2,
};

/// Width of the rail; a page narrower than `RAIL_MIN_PAGE` leaves it out (the table needs the room).
const RAIL_W: f32 = 236.0;
const RAIL_MIN_PAGE: f32 = 860.0;
const BASKET_H: f32 = 64.0;
/// A page shorter than this leaves the tray out.
const BASKET_MIN_PAGE_H: f32 = 340.0;

impl Gui {
    /// Draws the tray and the rail that `doc` asks for and returns the room that is left for the page.
    pub(super) fn rail_and_basket(
        &mut self,
        ui: &mut egui::Ui,
        mut area: Rect,
        doc: &Document,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) -> Rect {
        if let Some(basket) = &doc.basket {
            if area.height() >= BASKET_MIN_PAGE_H {
                let strip =
                    Rect::from_min_max(Pos2::new(area.left(), area.bottom() - BASKET_H), area.max);
                self.basket_tray(ui, strip, basket, p, out, hint);
                area.max.y -= BASKET_H;
            }
        }
        if let Some(rail) = &doc.rail {
            if area.width() >= RAIL_MIN_PAGE {
                let strip =
                    Rect::from_min_max(area.min, Pos2::new(area.left() + RAIL_W, area.bottom()));
                self.rail(ui, strip, rail, p, out);
                area.min.x += RAIL_W;
            }
        }
        area
    }

    fn rail(
        &mut self,
        ui: &mut egui::Ui,
        strip: Rect,
        rail: &DocNarrow,
        p: &Palette,
        out: &mut Vec<Outcome>,
    ) {
        ui.painter().rect_filled(strip, 0.0, p.side);
        ui.painter().line_segment(
            [strip.right_top(), strip.right_bottom()],
            Stroke::new(1.0, p.line),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(strip.shrink2(Vec2::new(12.0, 14.0)))
                .id_salt("doc-rail"),
            |ui| {
                ui.set_clip_rect(strip.intersect(ui.clip_rect()));
                self.narrow_block(ui, rail, p, out, 0.0);
            },
        );
    }

    fn basket_tray(
        &mut self,
        ui: &mut egui::Ui,
        strip: Rect,
        basket: &DocBasket,
        p: &Palette,
        out: &mut Vec<Outcome>,
        hint: &mut Option<String>,
    ) {
        ui.painter().rect_filled(strip, 0.0, p.side);
        ui.painter().line_segment(
            [strip.left_top(), strip.right_top()],
            Stroke::new(1.0, p.line),
        );
        let inner = strip.shrink2(Vec2::new(14.0, 9.0));
        ui.scope_builder(
            UiBuilder::new().max_rect(inner).id_salt("doc-basket"),
            |ui| {
                ui.set_clip_rect(strip.intersect(ui.clip_rect()));
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let filled = basket.slots.iter().flatten().count();
                    ui.label(
                        egui::RichText::new(format!("BASKET {filled}/{}", basket.slots.len()))
                            .strong()
                            .size(9.0)
                            .color(p.muted),
                    );
                    // The two buttons keep their room; the slots share the rest.
                    let buttons = 190.0;
                    let n = basket.slots.len().max(1) as f32;
                    let slot_w = ((ui.available_width() - buttons) / n - 8.0).clamp(80.0, 230.0);
                    for (k, slot) in basket.slots.iter().enumerate() {
                        self.basket_slot(ui, k, slot.as_ref(), basket, slot_w, p, out);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        self.doc_button(ui, &basket.compare, p, out, hint, true);
                        self.doc_button(ui, &basket.clear, p, out, hint, false);
                    });
                });
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn basket_slot(
        &mut self,
        ui: &mut egui::Ui,
        k: usize,
        slot: Option<&crate::gpu_viewer::ui::DocBasketSlot>,
        basket: &DocBasket,
        width: f32,
        p: &Palette,
        out: &mut Vec<Outcome>,
    ) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 44.0), Sense::hover());
        let radius = CornerRadius::same(9);
        let Some(slot) = slot else {
            ui.painter().rect_stroke(
                rect,
                radius,
                Stroke::new(1.0, p.line),
                egui::StrokeKind::Inside,
            );
            ui.painter().with_clip_rect(rect.shrink(6.0)).text(
                rect.center(),
                Align2::CENTER_CENTER,
                format!("+ {} {}", basket.empty, k + 1),
                FontId::proportional(11.0),
                p.muted,
            );
            return;
        };
        ui.painter().rect(
            rect,
            radius,
            with_alpha(p.accent, 22),
            Stroke::new(1.0, with_alpha(p.accent, 110)),
            egui::StrokeKind::Inside,
        );
        let badge = Rect::from_center_size(
            Pos2::new(rect.left() + 18.0, rect.center().y),
            Vec2::splat(20.0),
        );
        ui.painter()
            .rect_filled(badge, CornerRadius::same(6), p.accent);
        ui.painter().text(
            badge.center(),
            Align2::CENTER_CENTER,
            (k + 1).to_string(),
            FontId::monospace(11.0),
            p.on_accent,
        );
        let x_rect = Rect::from_center_size(
            Pos2::new(rect.right() - 14.0, rect.center().y),
            Vec2::splat(22.0),
        );
        let text_clip = ui.painter().with_clip_rect(Rect::from_min_max(
            Pos2::new(badge.right() + 8.0, rect.top()),
            Pos2::new(x_rect.left() - 2.0, rect.bottom()),
        ));
        // A narrow tray cuts the name; the whole slot says it on hover.
        ui.interact(rect, Id::new(("basket-slot", k)), Sense::hover())
            .on_hover_text(format!("{}\n{}", slot.name, slot.detail));
        let left = badge.right() + 8.0;
        text_clip.text(
            Pos2::new(left, rect.top() + 8.0),
            Align2::LEFT_TOP,
            &slot.name,
            FontId::proportional(12.0),
            p.text,
        );
        text_clip.text(
            Pos2::new(left, rect.top() + 25.0),
            Align2::LEFT_TOP,
            &slot.detail,
            FontId::proportional(10.0),
            p.dim,
        );
        let x = ui.interact(x_rect, Id::new(("basket-remove", k)), Sense::click());
        // Drawn, not typed: the proportional font has no cross glyph.
        let cross = Stroke::new(1.4, if x.hovered() { p.text } else { p.dim });
        let c = x_rect.center();
        ui.painter()
            .line_segment([c + Vec2::new(-4.0, -4.0), c + Vec2::new(4.0, 4.0)], cross);
        ui.painter()
            .line_segment([c + Vec2::new(-4.0, 4.0), c + Vec2::new(4.0, -4.0)], cross);
        self.targets
            .push((format!("Remove {} from the basket", slot.name), x_rect));
        if x.on_hover_text("Take out of the basket").clicked() {
            out.push(Outcome::Action(slot.remove));
        }
    }
}
