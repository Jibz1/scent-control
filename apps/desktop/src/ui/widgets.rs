//! The controls the screens share, drawn the way macOS draws them, with the colors and
//! sizes in `theme`. Task 10's sheets reuse them:
//!
//! - [`switch`]: on or off, such as power.
//! - [`segmented`]: one of a few choices, such as the intensity.
//! - [`grouped`] and [`row`]: a rounded box of label–value rows with hairlines
//!   between them, as in System Settings.
//! - [`status_dot`]: the 8 pt dot that says whether a diffuser answers.
//! - [`notice`] and [`notice_with`]: a tinted notice at the top of a pane.
//! - [`default_button`], [`plain_button`] and [`toolbar_button`]: push buttons. The
//!   default one, filled in the accent color, is the one Return presses.
//! - [`checkbox`]: for a choice that isn't a setting, such as a filter.
//!
//! Each reports its role, label and value to accessibility. Text in them follows the
//! capitalization rules: title case on buttons and segments, sentence case on labels.
//! The switch and the segmented control move only when the person changes them; a
//! change that comes from elsewhere (a diffuser's report) is shown at once.

use eframe::egui::accesskit::Role;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, Frame, Id, Margin, Rect, Response, RichText, Sense,
    Stroke, StrokeKind, TextStyle, TextWrapMode, Ui, WidgetInfo, WidgetText, WidgetType, pos2,
    vec2,
};

use super::theme::{BOX_RADIUS, CONTROL_RADIUS, NOTICE_RADIUS, Palette, headline, subheadline};

/// The regular macOS switch.
const SWITCH_SIZE: egui::Vec2 = vec2(38.0, 22.0);

/// The height of a push button and of a segmented control.
const CONTROL_HEIGHT: f32 = 22.0;

/// The narrowest a push button gets, so that a pair such as Cancel and Save line up.
const MIN_BUTTON_WIDTH: f32 = 72.0;

/// The space inside a grouped box on its left and right, and the least above and below
/// each row's content.
const BOX_PADDING_X: f32 = 12.0;
const ROW_PADDING_Y: f32 = 5.0;

/// How tall a row is at least: room for a 22 pt control, or for a value with a note
/// under it, with the content centered.
const ROW_MIN_HEIGHT: f32 = 40.0;

/// Where [`row`] counts the rows of the box it is in, to draw a hairline above all but
/// the first.
const ROW_COUNT: &str = "grouped_rows";

/// A hairline: one physical pixel.
fn hairline(ui: &Ui) -> f32 {
    1.0 / ui.pixels_per_point()
}

/// Moves a control's part to `target` over the style's animation time when the person
/// changed it this frame (`by_person`), and puts it there at once when the change came
/// from elsewhere.
fn motion(ui: &Ui, id: Id, target: f32, by_person: bool) -> f32 {
    let last = id.with("target");
    let moved_elsewhere = !by_person && ui.data(|d| d.get_temp::<f32>(last)) != Some(target);
    ui.data_mut(|d| d.insert_temp(last, target));
    let time = if moved_elsewhere {
        0.0
    } else {
        ui.style().animation_time
    };
    ui.ctx().animate_value_with_time(id, target, time)
}

/// The accent ring around a control that has the keyboard focus.
fn focus_ring(ui: &Ui, response: &Response, rect: Rect, radius: impl Into<CornerRadius>) {
    if response.has_focus() {
        let ring = Palette::tint(Palette::of(ui).accent, 50);
        ui.painter().rect_stroke(
            rect.expand(2.0),
            radius,
            Stroke::new(3.0, ring),
            StrokeKind::Middle,
        );
    }
}

/// An on/off switch. Clicking it flips `on`, and the response is then `changed`.
/// `label` names it to accessibility and is not drawn: put the switch in a [`row`]
/// with that label beside it.
pub fn switch(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let palette = Palette::of(ui);
    let (rect, mut response) = ui.allocate_exact_size(SWITCH_SIZE, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, *on, label));
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_role(Role::Switch));

    if ui.is_rect_visible(rect) {
        let how_on = motion(
            ui,
            response.id,
            f32::from(u8::from(*on)),
            response.changed(),
        );
        let off_fill = if ui.visuals().dark_mode {
            Color32::from_white_alpha(41)
        } else {
            Color32::from_black_alpha(23)
        };
        let fill = lerp_color(off_fill, palette.accent, how_on);
        let radius = rect.height() / 2.0;
        ui.painter()
            .rect_filled(rect, CornerRadius::from(radius), fill);
        let knob_radius = radius - 2.0;
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        let center = pos2(x, rect.center().y);
        let painter = ui.painter();
        painter.circle_filled(
            center + vec2(0.0, 0.5),
            knob_radius + 0.5,
            Color32::from_black_alpha(40),
        );
        painter.circle_filled(center, knob_radius, Color32::WHITE);
        focus_ring(ui, &response, rect, CornerRadius::from(radius));
    }
    response
}

/// A checkbox with `label` beside it, in sentence case, for a choice that isn't a
/// setting, such as a filter. Checked, it is filled in the accent color.
pub fn checkbox(ui: &mut Ui, checked: &mut bool, label: &str) -> Response {
    let palette = Palette::of(ui);
    let galley = WidgetText::from(label).into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Body,
    );
    let box_size = 14.0;
    let gap = 6.0;
    let size = vec2(
        box_size + gap + galley.size().x,
        galley.size().y.max(box_size),
    );
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, *checked, label));
    if ui.is_rect_visible(rect) {
        let mark = Rect::from_center_size(
            pos2(rect.left() + box_size / 2.0, rect.center().y),
            vec2(box_size, box_size),
        );
        let painter = ui.painter();
        if *checked {
            painter.rect_filled(mark, 3.5, palette.accent);
            let tick = [
                pos2(mark.left() + 3.5, mark.center().y + 0.5),
                pos2(mark.center().x - 1.0, mark.bottom() - 4.0),
                pos2(mark.right() - 3.5, mark.top() + 3.5),
            ];
            painter.line_segment([tick[0], tick[1]], Stroke::new(1.8, Color32::WHITE));
            painter.line_segment([tick[1], tick[2]], Stroke::new(1.8, Color32::WHITE));
        } else {
            let widgets = &ui.visuals().widgets.inactive;
            let edge = Stroke::new(0.5, palette.secondary_label);
            painter.rect(mark, 3.5, widgets.bg_fill, edge, StrokeKind::Inside);
        }
        let text_at = pos2(mark.right() + gap, rect.center().y - galley.size().y / 2.0);
        painter.galley(text_at, galley, palette.label);
        focus_ring(ui, &response, mark, 3.5);
    }
    response
}

fn lerp_color(from: Color32, to: Color32, t: f32) -> Color32 {
    from.lerp_to_gamma(to, t.clamp(0.0, 1.0))
}

/// What a [`segmented`] control did this frame.
pub struct Choice<T> {
    /// The whole control. It is `changed` when another segment was chosen.
    pub response: Response,
    /// The segment clicked, even the one already chosen: some choices are worth
    /// making again, such as an intensity to send again.
    pub clicked: Option<T>,
}

/// A segmented control: one of `items`, each a value and the title it shows. Clicking a
/// segment sets `value` to it. When `value` is none of them, no segment shows as chosen.
/// Each segment is a radio button to accessibility, with its title as the label.
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    value: &mut T,
    items: &[(T, &str)],
) -> Choice<T> {
    let palette = Palette::of(ui);
    let dark = ui.visuals().dark_mode;
    let galleys: Vec<_> = items
        .iter()
        .map(|(_, title)| {
            WidgetText::from(*title).into_galley(
                ui,
                Some(TextWrapMode::Extend),
                f32::INFINITY,
                TextStyle::Button,
            )
        })
        .collect();
    let widest = galleys
        .iter()
        .map(|galley| galley.size().x)
        .fold(0.0, f32::max);
    let segment_width = widest + 2.0 * ui.spacing().button_padding.x;
    let size = vec2(segment_width * items.len() as f32, CONTROL_HEIGHT);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::hover());
    let id = response.id;

    let enabled = ui.is_enabled();
    let mut clicked = None;
    let segment = |i: usize| {
        Rect::from_min_size(
            pos2(rect.left() + segment_width * i as f32, rect.top()),
            vec2(segment_width, rect.height()),
        )
    };
    for (i, (item, title)) in items.iter().enumerate() {
        let part = ui.interact(segment(i), id.with(i), Sense::click());
        if part.clicked() {
            clicked = Some(*item);
        }
        let chosen = clicked.map_or(*value == *item, |c| c == *item);
        part.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, enabled, chosen, *title));
        focus_ring(ui, &part, segment(i), CONTROL_RADIUS);
        response |= part;
    }
    if let Some(item) = clicked
        && *value != item
    {
        *value = item;
        response.mark_changed();
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let track = if dark {
            Color32::from_white_alpha(26)
        } else {
            Color32::from_black_alpha(15)
        };
        painter.rect_filled(rect, CONTROL_RADIUS, track);
        let chosen = items.iter().position(|(item, _)| item == value);
        if let Some(i) = chosen {
            let at = motion(ui, id, i as f32, response.changed());
            let pill = segment(0)
                .translate(vec2(segment_width * at, 0.0))
                .shrink(2.0);
            let (fill, edge) = if dark {
                (Color32::from_white_alpha(64), Stroke::NONE)
            } else {
                (
                    Color32::WHITE,
                    Stroke::new(0.5, Color32::from_black_alpha(30)),
                )
            };
            painter.rect(pill, CONTROL_RADIUS - 2, fill, edge, StrokeKind::Inside);
        }
        // Dividers between segments, except beside the chosen one.
        for i in 1..items.len() {
            if chosen.is_some_and(|c| c == i || c + 1 == i) {
                continue;
            }
            let x = segment(i).left();
            painter.vline(
                x,
                (rect.top() + 5.0)..=(rect.bottom() - 5.0),
                Stroke::new(1.0, palette.separator),
            );
        }
        for (i, galley) in galleys.into_iter().enumerate() {
            let at = Align2::CENTER_CENTER.align_size_within_rect(galley.size(), segment(i));
            painter.galley(at.min, galley, palette.label);
        }
    }
    Choice { response, clicked }
}

/// A grouped box, as in System Settings: `title` as a section heading above it (none
/// when empty), and the rows `add_rows` adds inside it. Put the [`row`]s straight in
/// it, not in a layout of their own, so that the hairlines between them are drawn.
pub fn grouped<R>(ui: &mut Ui, title: &str, add_rows: impl FnOnce(&mut Ui) -> R) -> R {
    let palette = Palette::of(ui);
    if !title.is_empty() {
        ui.label(
            RichText::new(title)
                .text_style(headline())
                .color(palette.label),
        );
    }
    let edge = if ui.visuals().dark_mode {
        Color32::from_white_alpha(13)
    } else {
        Color32::from_black_alpha(13)
    };
    Frame::new()
        .fill(palette.grouped)
        .stroke(Stroke::new(hairline(ui), edge))
        .corner_radius(BOX_RADIUS)
        .inner_margin(Margin::symmetric(BOX_PADDING_X as i8, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.data_mut(|d| d.insert_temp(ui.id().with(ROW_COUNT), 0_usize));
            add_rows(ui)
        })
        .inner
}

/// A row of a [`grouped`] box: `label` on the left and what `add_contents` adds on the
/// right, with a hairline above it unless it is the box's first row.
pub fn row<R>(ui: &mut Ui, label: &str, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let palette = Palette::of(ui);
    let count = ui.id().with(ROW_COUNT);
    let before = ui.data_mut(|d| {
        let rows = d.get_temp_mut_or_default::<usize>(count);
        *rows += 1;
        *rows - 1
    });
    if before > 0 {
        let y = ui.cursor().top();
        ui.painter().hline(
            ui.max_rect().x_range(),
            y,
            Stroke::new(hairline(ui), palette.separator),
        );
    }
    ui.add_space(ROW_PADDING_Y);
    let (_, inner) = egui::Sides::new()
        .height(ROW_MIN_HEIGHT - 2.0 * ROW_PADDING_Y)
        .shrink_left()
        .truncate()
        .show(
            ui,
            |ui| ui.label(RichText::new(label).color(palette.label)),
            add_contents,
        );
    ui.add_space(ROW_PADDING_Y);
    inner
}

/// A value with a note under it, both right-aligned, for the right side of a [`row`]:
/// a reading, and when and over what it came in. The note is Subheadline, in the
/// secondary label color.
pub fn value_with_note(ui: &mut Ui, value: RichText, note: Option<&str>) -> Response {
    let palette = Palette::of(ui);
    let one_line = |text: RichText, style: TextStyle| {
        WidgetText::from(text).into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, style)
    };
    let value = one_line(value, TextStyle::Body);
    let note = note.map(|note| {
        let text = RichText::new(note).color(palette.secondary_label);
        one_line(text, subheadline())
    });
    let note_size = note.as_ref().map_or(vec2(0.0, 0.0), |note| note.size());
    let size = vec2(
        value.size().x.max(note_size.x),
        value.size().y + note_size.y,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    response.widget_info(|| {
        let text = note.as_ref().map_or_else(
            || value.text().to_owned(),
            |note| format!("{}, {}", value.text(), note.text()),
        );
        WidgetInfo::labeled(WidgetType::Label, true, text)
    });
    let painter = ui.painter();
    let value_at = pos2(rect.right() - value.size().x, rect.top());
    let value_height = value.size().y;
    painter.galley(value_at, value, palette.label);
    if let Some(note) = note {
        let at = pos2(rect.right() - note.size().x, rect.top() + value_height);
        painter.galley(at, note, palette.secondary_label);
    }
    response
}

/// The 8 pt status dot in `color`, as tall as a line of text so that it lines up with
/// the words beside it. Color is never the only signal: `words` say the same thing, and
/// are the dot's value to accessibility.
pub fn status_dot(ui: &mut Ui, color: Color32, words: &str) -> Response {
    let height = ui.text_style_height(&TextStyle::Body);
    let (rect, response) = ui.allocate_exact_size(vec2(8.0, height), Sense::hover());
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::Image, true, "Status");
        info.current_text_value = Some(words.to_owned());
        info
    });
    ui.painter().circle_filled(rect.center(), 4.0, color);
    response
}

/// What a notice is about, which picks its color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// Orange: something needs the person's eye, such as a damaged save file.
    Warning,
    /// Red: something failed, such as a refused login.
    Error,
}

/// An inline notice: `text` in a rounded box tinted 12% with the kind's color.
pub fn notice(ui: &mut Ui, kind: NoticeKind, text: &str) -> Response {
    notice_with(ui, kind, text, |_| ()).1
}

/// A [`notice`] with buttons that `add_actions` adds on its right. Returns what that
/// returned, and the notice's response.
pub fn notice_with<R>(
    ui: &mut Ui,
    kind: NoticeKind,
    text: &str,
    add_actions: impl FnOnce(&mut Ui) -> R,
) -> (R, Response) {
    let palette = Palette::of(ui);
    let color = match kind {
        NoticeKind::Warning => palette.orange,
        NoticeKind::Error => palette.red,
    };
    let inner = Frame::new()
        .fill(Palette::tint(color, 12))
        .corner_radius(NOTICE_RADIUS)
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Sides::new()
                .shrink_left()
                .wrap()
                .show(
                    ui,
                    |ui| ui.label(RichText::new(text).color(palette.label)),
                    add_actions,
                )
                .1
        });
    (inner.inner, inner.response)
}

/// How a push button is filled.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ButtonStyle {
    /// Filled in the accent color with white text: the default action.
    Default,
    /// The usual push button.
    Plain,
    /// No frame until the pointer is over it, as in a toolbar.
    Toolbar,
}

/// The default push button: filled in the accent color, the action Return takes.
pub fn default_button(ui: &mut Ui, text: &str) -> Response {
    push_button(ui, text, ButtonStyle::Default)
}

/// A push button that isn't the default one, such as Cancel.
pub fn plain_button(ui: &mut Ui, text: &str) -> Response {
    push_button(ui, text, ButtonStyle::Plain)
}

/// A borderless button for the toolbar, such as Show Log.
pub fn toolbar_button(ui: &mut Ui, text: &str) -> Response {
    push_button(ui, text, ButtonStyle::Toolbar)
}

fn push_button(ui: &mut Ui, text: &str, style: ButtonStyle) -> Response {
    let palette = Palette::of(ui);
    let galley = WidgetText::from(text).into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Button,
    );
    let padding = ui.spacing().button_padding.x;
    let min_width = if style == ButtonStyle::Toolbar {
        0.0
    } else {
        MIN_BUTTON_WIDTH
    };
    let size = vec2(
        (galley.size().x + 2.0 * padding).max(min_width),
        CONTROL_HEIGHT,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, text));
    if ui.is_rect_visible(rect) {
        let pressed = response.is_pointer_button_down_on();
        let widgets = &ui.visuals().widgets;
        let (fill, stroke, text_color) = match style {
            ButtonStyle::Default => {
                let fill = if pressed {
                    palette.accent.gamma_multiply(0.85)
                } else {
                    palette.accent
                };
                (fill, Stroke::NONE, Color32::WHITE)
            }
            ButtonStyle::Plain => {
                let state = if pressed {
                    &widgets.active
                } else {
                    &widgets.inactive
                };
                (state.bg_fill, state.bg_stroke, palette.label)
            }
            ButtonStyle::Toolbar => {
                let fill = if pressed {
                    Palette::tint(palette.label, 12)
                } else if response.hovered() {
                    Palette::tint(palette.label, 6)
                } else {
                    Color32::TRANSPARENT
                };
                (fill, Stroke::NONE, palette.label)
            }
        };
        let painter = ui.painter();
        if style == ButtonStyle::Plain && !ui.visuals().dark_mode {
            // The light push button's shadow, a hairline under it.
            painter.rect_filled(
                rect.translate(vec2(0.0, 0.5)),
                CONTROL_RADIUS,
                Color32::from_black_alpha(20),
            );
        }
        painter.rect(rect, CONTROL_RADIUS, fill, stroke, StrokeKind::Inside);
        let at = Align2::CENTER_CENTER.align_size_within_rect(galley.size(), rect);
        painter.galley(at.min, galley, text_color);
        focus_ring(ui, &response, rect, CONTROL_RADIUS);
    }
    response
}

/// The toolbar's gear button, which opens Settings. egui can't draw SF Symbols, so the
/// gear is drawn here; `label` names it to accessibility.
pub fn gear_button(ui: &mut Ui, label: &str) -> Response {
    let palette = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, CONTROL_HEIGHT), Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    if ui.is_rect_visible(rect) {
        let fill = if response.is_pointer_button_down_on() {
            Palette::tint(palette.label, 12)
        } else if response.hovered() {
            Palette::tint(palette.label, 6)
        } else {
            Color32::TRANSPARENT
        };
        let painter = ui.painter();
        painter.rect_filled(rect, CONTROL_RADIUS, fill);
        let center = rect.center();
        let color = Palette::tint(palette.label, 90);
        let teeth = 8;
        for tooth in 0..teeth {
            let angle = std::f32::consts::TAU * tooth as f32 / teeth as f32;
            let direction = vec2(angle.cos(), angle.sin());
            painter.line_segment(
                [center + direction * 4.5, center + direction * 7.5],
                Stroke::new(2.6, color),
            );
        }
        painter.circle_stroke(center, 4.0, Stroke::new(2.2, color));
        focus_ring(ui, &response, rect, CONTROL_RADIUS);
    }
    response
}

#[cfg(test)]
mod tests {
    use eframe::egui::accesskit::{Role, Toggled};
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Route {
        Auto,
        Wifi,
        Bluetooth,
    }

    const ROUTES: [(Route, &str); 3] = [
        (Route::Auto, "Auto"),
        (Route::Wifi, "WiFi"),
        (Route::Bluetooth, "Bluetooth"),
    ];

    #[test]
    fn segmented_selects_the_clicked_segment() {
        let mut harness = Harness::new_ui_state(
            |ui, route: &mut Route| {
                segmented(ui, route, &ROUTES);
            },
            Route::Auto,
        );
        let auto = harness.get_by_label("Auto");
        assert_eq!(auto.accesskit_node().role(), Role::RadioButton);
        assert_eq!(auto.accesskit_node().toggled(), Some(Toggled::True));

        harness.get_by_label("WiFi").click();
        harness.run();
        assert_eq!(*harness.state(), Route::Wifi);
        let wifi = harness.get_by_label("WiFi");
        assert_eq!(wifi.accesskit_node().toggled(), Some(Toggled::True));
        let auto = harness.get_by_label("Auto");
        assert_eq!(auto.accesskit_node().toggled(), Some(Toggled::False));
    }

    #[test]
    fn a_click_on_the_chosen_segment_is_still_a_click() {
        // The intensity buttons send again when the chosen one is clicked.
        let mut harness = Harness::new_ui_state(
            |ui, clicks: &mut Vec<Route>| {
                let mut route = Route::Wifi;
                if let Some(clicked) = segmented(ui, &mut route, &ROUTES).clicked {
                    clicks.push(clicked);
                }
            },
            Vec::new(),
        );
        harness.get_by_label("WiFi").click();
        harness.run();
        assert_eq!(harness.state(), &[Route::Wifi]);
    }

    #[test]
    fn the_switch_flips_and_says_so() {
        let mut harness = Harness::new_ui_state(
            |ui, on: &mut bool| {
                switch(ui, on, "Power");
            },
            false,
        );
        let node = harness.get_by_label("Power");
        assert_eq!(node.accesskit_node().role(), Role::Switch);
        assert_eq!(node.accesskit_node().toggled(), Some(Toggled::False));

        harness.get_by_label("Power").click();
        harness.run();
        assert!(*harness.state());
        let node = harness.get_by_label("Power");
        assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True));
    }

    #[test]
    fn the_status_dot_says_its_status_in_words() {
        let harness = Harness::new_ui(|ui| {
            let palette = Palette::of(ui);
            status_dot(ui, palette.green, "Answering");
        });
        let dot = harness.get_by_label("Status");
        assert_eq!(dot.accesskit_node().value().as_deref(), Some("Answering"));
    }

    #[test]
    fn buttons_report_their_clicks() {
        let mut harness = Harness::new_ui_state(
            |ui, clicked: &mut Vec<&'static str>| {
                if default_button(ui, "Save").clicked() {
                    clicked.push("Save");
                }
                if plain_button(ui, "Cancel").clicked() {
                    clicked.push("Cancel");
                }
            },
            Vec::new(),
        );
        assert_eq!(
            harness.get_by_label("Save").accesskit_node().role(),
            Role::Button
        );
        harness.get_by_label("Cancel").click();
        harness.run();
        harness.get_by_label("Save").click();
        harness.run();
        assert_eq!(harness.state(), &["Cancel", "Save"]);
    }
}
