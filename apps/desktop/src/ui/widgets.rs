//! The controls the screens share, drawn the way macOS draws them, with the colors and
//! sizes in `theme`. Task 10's sheets reuse them:
//!
//! - [`switch`]: on or off, such as power.
//! - [`segmented`]: one of a few choices, such as the intensity; it hands back a
//!   [`Choice`]. [`segmented_wide`] fills the width it is given, as in the hero.
//! - [`grouped`] and [`row`]: a rounded box of label–value rows with hairlines
//!   between them, as in System Settings. [`grouped_with`] adds something at the end of
//!   the box's heading, such as Read Status; [`value_with_note`] is a row's value with
//!   its age and source under it; [`free_row`] is a row laid out by its caller, such as
//!   the Today strip.
//! - [`status_dot`]: the 8 pt dot that says whether a diffuser answers.
//! - [`notice`] and [`notice_with`]: a tinted notice at the top of a pane, by
//!   [`NoticeKind`].
//! - [`default_button`], [`plain_button`] and [`toolbar_button`]: push buttons. The
//!   default one, filled in the accent color, is the one Return presses.
//!   [`gear_button`] is the toolbar's way to Settings.
//! - [`text_field`]: a one-line [`Field`], such as a diffuser's name. [`combo_field`] is
//!   one with a list to pick from as well, as a Mac combo box, such as its room.
//! - [`checkbox`]: for a choice that isn't a setting, such as a filter.
//! - [`day_toggles`]: the days of the week, S M T W T F S, each on or off.
//! - [`secondary`] and [`footnote`]: Subheadline text in the secondary label color, the
//!   second under a grouped box, lined up with its rows at [`BOX_PADDING_X`].
//!   [`problem`] is a footnote in red, for what is wrong with the box above it.
//! - [`sheet`]: a modal panel that hangs from the top of the window, as a Mac sheet
//!   does; [`alert`] is one in the middle of the window, such as a confirmation.
//! - [`empty_state`]: a glyph, one Title 2 line, one sentence and one default button,
//!   for a pane with nothing to show.
//! - [`one_line`] lays text out on one line, cut short to a width; [`hairline`] is the
//!   width of a one-pixel line.
//!
//! Each reports its role, label and value to accessibility. Text in them follows the
//! capitalization rules: title case on buttons and segments, sentence case on labels.
//! The switch and the segmented control move only when the person changes them; a
//! change that comes from elsewhere (a diffuser's report) is shown at once.

use eframe::egui::accesskit::Role;
use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontSelection, Frame, Galley, Id, Margin, Popup, Rect,
    RectAlign, Response, RichText, Sense, Stroke, StrokeKind, TextEdit, TextStyle, TextWrapMode,
    Ui, UiBuilder, WidgetInfo, WidgetText, WidgetType, pos2, vec2,
};

use super::theme::{
    BOX_RADIUS, CONTROL_RADIUS, GRID, MARGIN, NOTICE_RADIUS, Palette, TOOLBAR_HEIGHT, headline,
    subheadline, title2,
};

/// The regular macOS switch.
const SWITCH_SIZE: egui::Vec2 = vec2(38.0, 22.0);

/// The height of a push button and of a segmented control.
const CONTROL_HEIGHT: f32 = 22.0;

/// The narrowest a push button gets, so that a pair such as Cancel and Save line up.
pub const MIN_BUTTON_WIDTH: f32 = 72.0;

/// The width of each of the [`day_toggles`], and the gap between them.
const DAY_WIDTH: f32 = 26.0;
const DAY_GAP: f32 = 2.0;

/// The days of the week, from Sunday (0), as [`day_toggles`] shows and names them.
const DAYS: [(&str, &str); 7] = [
    ("S", "Sunday"),
    ("M", "Monday"),
    ("T", "Tuesday"),
    ("W", "Wednesday"),
    ("T", "Thursday"),
    ("F", "Friday"),
    ("S", "Saturday"),
];

/// The space inside a grouped box on its left and right. A row's text, the hairlines
/// between rows and a [`footnote`] under the box all start this far in.
pub const BOX_PADDING_X: f32 = 16.0;

/// How tall a row is at least, on the 8 pt grid: a 22 pt control, or a value with a note
/// under it, centered with [`ROW_PADDING_Y`] at least above and below.
const ROW_MIN_HEIGHT: f32 = 40.0;
const ROW_PADDING_Y: f32 = 4.0;

/// Where [`row`] counts the rows of the box it is in, to draw a hairline above all but
/// the first.
const ROW_COUNT: &str = "grouped_rows";

/// The space inside a text field around its text, as egui's own; and the width of the
/// chevron at the end of a [`combo_field`].
const FIELD_MARGIN: Margin = Margin::symmetric(4, 2);
const CHEVRON_WIDTH: f32 = 20.0;

/// Where a [`combo_field`] keeps whether its list is open, under the field's ID.
const CHOICES: &str = "choices";

/// A hairline: one physical pixel.
pub fn hairline(ui: &Ui) -> f32 {
    1.0 / ui.pixels_per_point()
}

/// `text` laid out on one line in `style`, cut short with "…" past `width` (none when it
/// is infinite).
pub fn one_line(ui: &Ui, text: impl Into<WidgetText>, style: TextStyle, width: f32) -> Arc<Galley> {
    text.into().into_galley(
        ui,
        Some(TextWrapMode::Truncate),
        width,
        FontSelection::Style(style),
    )
}

/// Subheadline text in the secondary label color, wrapped: notes, empty states, and
/// "Sending…".
pub fn secondary(ui: &mut Ui, text: &str) -> Response {
    let color = Palette::of(ui).secondary_label;
    ui.add(egui::Label::new(RichText::new(text).text_style(subheadline()).color(color)).wrap())
}

/// A [`secondary`] note under a grouped box, lined up with the text of its rows, such as
/// the expected-state line under "Status".
pub fn footnote(ui: &mut Ui, text: &str) -> Response {
    ui.add_space(GRID / 2.0);
    ui.horizontal(|ui| {
        ui.add_space(BOX_PADDING_X);
        secondary(ui, text)
    })
    .inner
}

/// A [`footnote`] in the red of an error: what is wrong with the box above it, such as a
/// time that ends before it starts.
pub fn problem(ui: &mut Ui, text: &str) -> Response {
    let color = Palette::of(ui).red;
    ui.add_space(GRID / 2.0);
    ui.horizontal(|ui| {
        ui.add_space(BOX_PADDING_X);
        ui.add(egui::Label::new(RichText::new(text).text_style(subheadline()).color(color)).wrap())
    })
    .inner
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

/// A one-line text field for a [`row`]: what accessibility calls it (`label`, which isn't
/// drawn, as the row shows it), what it shows while it is empty (`hint`), how wide it is,
/// and how many characters it takes.
pub struct Field<'a> {
    pub id: Id,
    pub label: &'a str,
    pub hint: &'a str,
    pub width: f32,
    pub max_chars: usize,
}

impl Field<'_> {
    fn edit<'t>(&self, text: &'t mut String) -> TextEdit<'t> {
        TextEdit::singleline(text)
            .id(self.id)
            .hint_text(self.hint)
            .desired_width(self.width)
            .char_limit(self.max_chars)
    }
}

/// A [`Field`] holding `text`. To accessibility it is a text input named by the field's
/// label, its value the text and its placeholder the hint.
pub fn text_field(ui: &mut Ui, text: &mut String, field: &Field<'_>) -> Response {
    let response = ui.add(field.edit(text));
    let label = field.label;
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_label(label));
    response
}

/// What a [`combo_field`] did this frame.
pub struct Combo {
    /// The text field.
    pub field: Response,
    /// What was picked from the list, which is now in the field.
    pub picked: Option<String>,
}

/// A combo box, as on the Mac: a [`text_field`] that takes any text, with a chevron at its
/// trailing end that lists `choices`, the one in the field checked. Picking one puts it
/// in the field. The chevron is a button named `chooser`, which says whether the list is
/// open. With no choices it is a plain text field.
pub fn combo_field(
    ui: &mut Ui,
    text: &mut String,
    field: &Field<'_>,
    choices: &[String],
    chooser: &str,
) -> Combo {
    if choices.is_empty() {
        let field = text_field(ui, text, field);
        return Combo {
            field,
            picked: None,
        };
    }
    let palette = Palette::of(ui);
    // The text stops short of the chevron.
    let margin = Margin {
        right: FIELD_MARGIN.right + CHEVRON_WIDTH as i8,
        ..FIELD_MARGIN
    };
    let response = ui.add(field.edit(text).margin(margin));
    let label = field.label;
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_label(label));

    let menu = field.id.with(CHOICES);
    let open = Popup::is_id_open(ui.ctx(), menu);
    let rect = Rect::from_min_max(
        pos2(response.rect.right() - CHEVRON_WIDTH, response.rect.top()),
        response.rect.max,
    );
    // Over the field's end, so that it takes the clicks there.
    let button = ui.interact(rect, field.id.with("chevron"), Sense::click());
    let enabled = ui.is_enabled();
    button.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, chooser));
    ui.ctx()
        .accesskit_node_builder(button.id, |node| node.set_expanded(open));
    if ui.is_rect_visible(rect) {
        let color = if !enabled {
            palette.gray
        } else if open || button.hovered() {
            palette.label
        } else {
            palette.secondary_label
        };
        let center = rect.center();
        let stroke = Stroke::new(1.5, color);
        let painter = ui.painter();
        painter.line_segment(
            [center + vec2(-3.5, -1.75), center + vec2(0.0, 1.75)],
            stroke,
        );
        painter.line_segment(
            [center + vec2(0.0, 1.75), center + vec2(3.5, -1.75)],
            stroke,
        );
        focus_ring(ui, &button, rect.shrink(2.0), CONTROL_RADIUS);
    }

    let mut picked = None;
    let width = response.rect.width();
    Popup::menu(&button)
        .id(menu)
        .anchor(response.rect)
        .align(RectAlign::BOTTOM_START)
        .show(|ui| {
            // As wide as the field, as a Mac combo box's list is.
            ui.set_min_width(width - ui.style().spacing.menu_margin.sum().x);
            for choice in choices {
                if ui.selectable_label(choice == text, choice).clicked() {
                    picked = Some(choice.clone());
                }
            }
        });
    if let Some(choice) = &picked {
        text.clone_from(choice);
    }
    Combo {
        field: response,
        picked,
    }
}

/// Whether the list of the [`combo_field`] whose field is `field_id` is open, so that a
/// sheet can leave Escape to it.
pub fn combo_open(ctx: &egui::Context, field_id: Id) -> bool {
    Popup::is_id_open(ctx, field_id.with(CHOICES))
}

/// Opens the list of the [`combo_field`] whose field is `field_id`, for the snapshots.
#[cfg(test)]
pub fn open_combo(ctx: &egui::Context, field_id: Id) {
    Popup::open_id(ctx, field_id.with(CHOICES));
}

/// A checkbox with `label` beside it, in sentence case, for a choice that isn't a
/// setting, such as a filter. Checked, it is filled in the accent color.
pub fn checkbox(ui: &mut Ui, checked: &mut bool, label: &str) -> Response {
    let palette = Palette::of(ui);
    let galley = one_line(ui, label, TextStyle::Body, f32::INFINITY);
    let box_size = 14.0;
    let gap = GRID;
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

/// The days of the week as seven toggle buttons, S M T W T F S, for the days a schedule
/// runs on: `days` holds the chosen ones, 0 for Sunday to 6 for Saturday, in order.
/// Clicking a day adds or removes it, and the response is then `changed`. A chosen day
/// is filled in the accent color. To accessibility it is a group named `label`, and each
/// day a checkbox in it named for the day.
pub fn day_toggles(ui: &mut Ui, days: &mut Vec<u8>, label: &str) -> Response {
    let palette = Palette::of(ui);
    let size = vec2(
        DAY_WIDTH * DAYS.len() as f32 + DAY_GAP * (DAYS.len() - 1) as f32,
        CONTROL_HEIGHT,
    );
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::hover());
    let id = response.id;
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, enabled, label));
    let group = ui.new_child(UiBuilder::new().max_rect(rect).accessibility_parent(id));
    let mut changed = false;
    for (day, (letter, name)) in (0_u8..).zip(DAYS) {
        let part_rect = Rect::from_min_size(
            rect.min + vec2(f32::from(day) * (DAY_WIDTH + DAY_GAP), 0.0),
            vec2(DAY_WIDTH, CONTROL_HEIGHT),
        );
        let part = group.interact(part_rect, id.with(day), Sense::click());
        if part.clicked() {
            match days.iter().position(|&chosen| chosen == day) {
                Some(at) => {
                    days.remove(at);
                }
                None => {
                    days.push(day);
                    days.sort_unstable();
                }
            }
            changed = true;
        }
        let on = days.contains(&day);
        part.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, on, name));
        if ui.is_rect_visible(part_rect) {
            let widgets = &ui.visuals().widgets.inactive;
            let (fill, edge, text) = if on {
                (palette.accent, Stroke::NONE, Color32::WHITE)
            } else {
                (widgets.bg_fill, widgets.bg_stroke, palette.label)
            };
            let painter = ui.painter();
            painter.rect(part_rect, CONTROL_RADIUS, fill, edge, StrokeKind::Inside);
            let galley = one_line(ui, letter, TextStyle::Button, f32::INFINITY);
            let at = Align2::CENTER_CENTER.align_size_within_rect(galley.size(), part_rect);
            painter.galley(at.min, galley, text);
            focus_ring(ui, &part, part_rect, CONTROL_RADIUS);
        }
        response |= part;
    }
    if changed {
        response.mark_changed();
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
/// To accessibility it is a radio group named `label`, as the switch is, and each
/// segment a radio button in it with its title as the label.
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    value: &mut T,
    items: &[(T, &str)],
    label: &str,
) -> Choice<T> {
    segmented_in(ui, value, items, label, None)
}

/// A [`segmented`] control as wide as the room it is given and `height` tall, each
/// segment an equal share: the hero's intensity.
pub fn segmented_wide<T: Copy + PartialEq>(
    ui: &mut Ui,
    value: &mut T,
    items: &[(T, &str)],
    label: &str,
    height: f32,
) -> Choice<T> {
    let width = ui.available_width();
    segmented_in(ui, value, items, label, Some(vec2(width, height)))
}

/// A segmented control `size` big, or just big enough for its titles.
fn segmented_in<T: Copy + PartialEq>(
    ui: &mut Ui,
    value: &mut T,
    items: &[(T, &str)],
    label: &str,
    size: Option<egui::Vec2>,
) -> Choice<T> {
    let palette = Palette::of(ui);
    let dark = ui.visuals().dark_mode;
    let galleys: Vec<_> = items
        .iter()
        .map(|(_, title)| one_line(ui, *title, TextStyle::Button, f32::INFINITY))
        .collect();
    let widest = galleys
        .iter()
        .map(|galley| galley.size().x)
        .fold(0.0, f32::max);
    let size = size.unwrap_or_else(|| {
        let segment = widest + 2.0 * ui.spacing().button_padding.x;
        vec2(segment * items.len() as f32, CONTROL_HEIGHT)
    });
    let segment_width = size.x / items.len() as f32;
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::hover());
    let id = response.id;
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::RadioGroup, enabled, label));
    // The segments are the group's children to accessibility.
    let group = ui.new_child(UiBuilder::new().max_rect(rect).accessibility_parent(id));

    let mut clicked = None;
    let segment = |i: usize| {
        Rect::from_min_size(
            pos2(rect.left() + segment_width * i as f32, rect.top()),
            vec2(segment_width, rect.height()),
        )
    };
    for (i, (item, title)) in items.iter().enumerate() {
        let part = group.interact(segment(i), id.with(i), Sense::click());
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
    grouped_with(ui, title, |_| {}, add_rows)
}

/// A [`grouped`] box with what `add_accessory` adds at the trailing edge of its section
/// heading, such as a button that acts on the whole section.
pub fn grouped_with<R>(
    ui: &mut Ui,
    title: &str,
    add_accessory: impl FnOnce(&mut Ui),
    add_rows: impl FnOnce(&mut Ui) -> R,
) -> R {
    let palette = Palette::of(ui);
    if !title.is_empty() {
        egui::Sides::new().show(
            ui,
            |ui| {
                ui.label(
                    RichText::new(title)
                        .text_style(headline())
                        .color(palette.label),
                )
            },
            add_accessory,
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

/// A row of a [`grouped`] box that its caller lays out across the whole width, such as
/// the Today strip, with the hairline above it that a [`row`] has.
pub fn free_row<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
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
    ui.add_space(3.0 * ROW_PADDING_Y);
    let inner = add_contents(ui);
    ui.add_space(3.0 * ROW_PADDING_Y);
    inner
}

/// A value with a note under it, both right-aligned, for the right side of a [`row`]:
/// a reading, and when and over what it came in. The note is Subheadline, in the
/// secondary label color.
pub fn value_with_note(ui: &mut Ui, value: RichText, note: Option<&str>) -> Response {
    let palette = Palette::of(ui);
    let value = one_line(ui, value, TextStyle::Body, f32::INFINITY);
    let note = note.map(|note| {
        let text = RichText::new(note).color(palette.secondary_label);
        one_line(ui, text, subheadline(), f32::INFINITY)
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

/// An empty state, centered in the room it has: a glyph that `draw_glyph` paints in the
/// square it is given (in the secondary color, on the window's), one Title 2 line, one
/// sentence, and one default button. True when the button was clicked.
pub fn empty_state(
    ui: &mut Ui,
    draw_glyph: impl FnOnce(&egui::Painter, Rect, Color32, Color32),
    title: &str,
    sentence: &str,
    button: &str,
) -> bool {
    let palette = Palette::of(ui);
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space(12.0 * GRID);
        let (rect, _) = ui.allocate_exact_size(vec2(96.0, 96.0), Sense::hover());
        draw_glyph(ui.painter(), rect, palette.gray, palette.window);
        ui.add_space(2.0 * GRID);
        ui.label(
            RichText::new(title)
                .text_style(title2())
                .color(palette.label),
        );
        ui.add_space(GRID / 2.0);
        ui.label(RichText::new(sentence).color(palette.secondary_label));
        ui.add_space(2.0 * GRID);
        clicked = default_button(ui, button).clicked();
    });
    clicked
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
        .inner_margin(Margin::symmetric(BOX_PADDING_X as i8, GRID as i8))
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

/// A sheet, as on the Mac: a modal panel `width` wide that hangs from the top of the
/// window's content, under the toolbar, over a light dim that keeps the window behind it
/// from being used. What `add_contents` adds is inside the window's margins. A click
/// outside doesn't close it, as it doesn't a Mac sheet: its own buttons and keys do.
pub fn sheet<R>(
    ctx: &egui::Context,
    id: Id,
    width: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    modal(
        ctx,
        id,
        width,
        (Align2::CENTER_TOP, vec2(0.0, TOOLBAR_HEIGHT)),
        add_contents,
    )
}

/// An alert: as a [`sheet`], in the middle of the window, over whatever is open. For a
/// question such as "Replace Bedroom's stored schedule?".
pub fn alert<R>(
    ctx: &egui::Context,
    id: Id,
    width: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    modal(
        ctx,
        id,
        width,
        (Align2::CENTER_CENTER, egui::Vec2::ZERO),
        add_contents,
    )
}

fn modal<R>(
    ctx: &egui::Context,
    id: Id,
    width: f32,
    (anchor, offset): (Align2, egui::Vec2),
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let style = ctx.global_style();
    let dark = style.visuals.dark_mode;
    let palette = Palette::for_dark(dark);
    let (dim, edge) = if dark {
        (Color32::from_black_alpha(60), Color32::from_white_alpha(26))
    } else {
        (Color32::from_black_alpha(20), Color32::from_black_alpha(38))
    };
    let frame = Frame::new()
        .fill(palette.window)
        .stroke(Stroke::new(1.0 / ctx.pixels_per_point(), edge))
        .corner_radius(BOX_RADIUS)
        .inner_margin(MARGIN)
        .shadow(style.visuals.window_shadow);
    egui::Modal::new(id)
        .area(egui::Modal::default_area(id).anchor(anchor, offset))
        .backdrop_color(dim)
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(width - 2.0 * MARGIN);
            add_contents(ui)
        })
        .inner
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
    let galley = one_line(ui, text, TextStyle::Button, f32::INFINITY);
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
        // A default button that can't be pressed is drawn as any other, as on the Mac:
        // a dim accent color would read as one that can.
        let style = if style == ButtonStyle::Default && !ui.is_enabled() {
            ButtonStyle::Plain
        } else {
            style
        };
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
            ButtonStyle::Toolbar => (
                borderless_fill(&palette, &response),
                Stroke::NONE,
                palette.label,
            ),
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

/// What a borderless button is filled with: nothing until the pointer is over it, then
/// a light tint of the label color, darker while pressed.
fn borderless_fill(palette: &Palette, response: &Response) -> Color32 {
    if response.is_pointer_button_down_on() {
        Palette::tint(palette.label, 12)
    } else if response.hovered() {
        Palette::tint(palette.label, 6)
    } else {
        Color32::TRANSPARENT
    }
}

/// The toolbar's gear button, which opens Settings. egui can't draw SF Symbols, so the
/// gear is drawn here; `label` names it to accessibility.
pub fn gear_button(ui: &mut Ui, label: &str) -> Response {
    let palette = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, CONTROL_HEIGHT), Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CONTROL_RADIUS, borderless_fill(&palette, &response));
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
                segmented(ui, route, &ROUTES, "Send Over");
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
    fn the_segments_belong_to_a_group_the_label_names() {
        let harness = Harness::new_ui_state(
            |ui, route: &mut Route| {
                segmented(ui, route, &ROUTES, "Send Over");
            },
            Route::Auto,
        );
        let group = harness.get_by_role_and_label(Role::RadioGroup, "Send Over");
        let group = group.accesskit_node().id();
        for title in ["Auto", "WiFi", "Bluetooth"] {
            let segment = harness.get_by_label(title);
            let in_group =
                std::iter::successors(segment.accesskit_node().parent(), |node| node.parent())
                    .any(|node| node.id() == group);
            assert!(in_group, "{title}");
        }
    }

    #[test]
    fn a_click_on_the_chosen_segment_is_still_a_click() {
        // The intensity buttons send again when the chosen one is clicked.
        let mut harness = Harness::new_ui_state(
            |ui, clicks: &mut Vec<Route>| {
                let mut route = Route::Wifi;
                if let Some(clicked) = segmented(ui, &mut route, &ROUTES, "Send Over").clicked {
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
    fn day_toggles_add_and_remove_days_in_order() {
        let mut harness = Harness::new_ui_state(
            |ui, days: &mut Vec<u8>| {
                day_toggles(ui, days, "Days");
            },
            vec![1, 5],
        );
        let monday = harness.get_by_label("Monday");
        assert_eq!(monday.accesskit_node().role(), Role::CheckBox);
        assert_eq!(monday.accesskit_node().toggled(), Some(Toggled::True));
        let sunday = harness.get_by_label("Sunday");
        assert_eq!(sunday.accesskit_node().toggled(), Some(Toggled::False));

        harness.get_by_label("Saturday").click();
        harness.run();
        harness.get_by_label("Sunday").click();
        harness.run();
        harness.get_by_label("Monday").click();
        harness.run();
        assert_eq!(harness.state(), &[0, 5, 6]);
        let saturday = harness.get_by_label("Saturday");
        assert_eq!(saturday.accesskit_node().toggled(), Some(Toggled::True));
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
