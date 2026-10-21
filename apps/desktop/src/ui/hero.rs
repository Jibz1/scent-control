//! The diffusion hero: the card at the top of the open diffuser that answers the one
//! question the owner glances for (is it diffusing, and how strongly?) and changes it.
//!
//! The diffuser's glyph with one to three wisps of mist rising for its level (accent while
//! it diffuses, grey when it's off), the state in words at 26 pt, the intensity under it,
//! where that came from, the power switch at the trailing edge, and the intensity across
//! the card's width. What a command on its way set is shown, with "Sending…" in orange,
//! so nothing is shown twice. Only the wisps move, easing when the level changes.
//!
//! To accessibility the card is one element ("Bedroom, diffusing at Medium intensity"),
//! with the switch and the segments as controls of their own; the glyph has no node.

use eframe::egui::accesskit::Role;
use eframe::egui::{
    Color32, FontSelection, Frame, Id, Margin, Rect, RichText, Sense, Stroke, TextWrapMode, Ui,
    UiBuilder, WidgetInfo, WidgetText, WidgetType, pos2, vec2,
};

use super::format::{Model, intensity};
use super::glyphs::{self, Ink};
use super::theme::{
    GRID, HERO_RADIUS, MOTION_SECS, Palette, hero as hero_text, subheadline, title3,
};
use super::widgets::{hairline, one_line, segmented_wide, switch};

/// The three intensities and what the segments call them.
const LEVELS: [(Option<u8>, &str); 3] = [(Some(1), "Low"), (Some(2), "Medium"), (Some(3), "High")];

/// The glyph's square, and the space between its ink and the words.
const GLYPH: f32 = 72.0;
const GLYPH_GAP: f32 = 2.0 * GRID;
/// The card's padding.
const PADDING: i8 = 20;

/// The intensity control: a large control, taller than the others.
const SEGMENTS_HEIGHT: f32 = 28.0;

/// What the hero shows, worked out from the diffuser and what is on its way to it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeroState {
    /// What the diffuser is called on screen.
    pub name: String,
    pub model: Option<Model>,
    /// On or off: what the person set and is on its way, else what was sent and not yet
    /// reported on, else what it reported. `None` when there is nothing to go on.
    pub power: Option<bool>,
    /// The intensity, 1 to 3, the same way.
    pub level: Option<u8>,
    /// A command is on its way.
    pub sending: bool,
    /// A command went out and the diffuser hasn't reported on it yet.
    pub pending: bool,
    /// The power it reported is more than 5 minutes old.
    pub stale: bool,
    /// The note says what last failed.
    pub failed: bool,
    /// Where the state comes from ("Reported just now over WiFi", "Sent, waiting for the
    /// diffuser"), how a command on its way is going, or what last failed.
    pub note: String,
}

impl HeroState {
    pub fn diffusing(&self) -> bool {
        self.power == Some(true)
    }

    /// The state at 26 pt.
    pub fn state_words(&self) -> &'static str {
        if self.sending {
            return "Sending…";
        }
        match self.power {
            Some(true) => "Diffusing",
            Some(false) => "Off",
            None => "Not reported yet",
        }
    }

    /// Orange while a command is on its way; the secondary color when the state is
    /// old or unknown.
    pub fn state_color(&self, palette: &Palette) -> Color32 {
        if self.sending {
            palette.orange
        } else if self.stale || self.power.is_none() {
            palette.secondary_label
        } else {
            palette.label
        }
    }

    /// The line under the state: "Medium intensity". Off, a diffuser reports no
    /// intensity (0), so it says it isn't diffusing.
    pub fn intensity_words(&self) -> String {
        match (self.level, self.power) {
            (Some(level @ 1..=3), _) => format!("{} intensity", intensity(i64::from(level))),
            (_, Some(false)) => "Not diffusing".to_owned(),
            _ => "Intensity not reported".to_owned(),
        }
    }

    /// The note in red when it says what failed, in orange when what was sent is waited
    /// for.
    pub fn note_color(&self, palette: &Palette) -> Color32 {
        if self.failed {
            palette.red
        } else if self.pending {
            palette.orange
        } else {
            palette.secondary_label
        }
    }

    /// The card as one sentence, for accessibility: "Bedroom, diffusing at Medium
    /// intensity", "Bedroom, off", "Bedroom, sending: diffusing at High intensity".
    pub fn sentence(&self) -> String {
        let doing = match (self.power, self.level) {
            (Some(true), Some(level @ 1..=3)) => {
                format!("diffusing at {} intensity", intensity(i64::from(level)))
            }
            (Some(true), _) => "diffusing".to_owned(),
            (Some(false), _) => "off".to_owned(),
            (None, _) => "not reported yet".to_owned(),
        };
        if self.sending {
            format!("{}, sending: {doing}", self.name)
        } else {
            format!("{}, {doing}", self.name)
        }
    }

    /// How many wisps of mist: the level, whether it diffuses or not (they are grey when
    /// it doesn't).
    fn wisps(&self) -> f32 {
        self.level
            .filter(|level| (1..=3).contains(level))
            .map_or(0.0, f32::from)
    }
}

/// What the person changed in the hero this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Changed {
    pub power: Option<bool>,
    /// A segment clicked, even the one already chosen: an intensity can be sent again.
    pub level: Option<u8>,
}

/// The hero card for `hero`, the full width of the column. `id` tells one diffuser's
/// card from another's, so that the wisps don't ease when another diffuser is opened.
pub fn show(ui: &mut Ui, hero: &HeroState, id: Id) -> Changed {
    let palette = Palette::of(ui);
    let edge = if ui.visuals().dark_mode {
        Color32::from_white_alpha(13)
    } else {
        Color32::from_black_alpha(13)
    };
    let mut changed = Changed::default();
    Frame::new()
        .fill(palette.grouped)
        .stroke(Stroke::new(hairline(ui), edge))
        .corner_radius(HERO_RADIUS)
        .inner_margin(Margin::same(PADDING))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let width = ui.available_width();
            // The words first, to know how tall the top of the card is: the note wraps,
            // so that what failed and how to fix it is never cut short.
            let scale = GLYPH / glyphs::UNIT;
            let [ink_left, ink_right] = glyphs::INK_X;
            let switch_size = vec2(38.0, 22.0);
            let text_offset = (ink_right - ink_left) * scale + GLYPH_GAP;
            let text_width = (width - text_offset - switch_size.x - GRID).max(0.0);
            let state = one_line(
                ui,
                RichText::new(hero.state_words()).color(hero.state_color(&palette)),
                hero_text(),
                text_width,
            );
            let level = one_line(
                ui,
                RichText::new(hero.intensity_words()).color(palette.secondary_label),
                title3(),
                text_width,
            );
            let note = WidgetText::from(RichText::new(&hero.note).color(hero.note_color(&palette)))
                .into_galley(
                    ui,
                    Some(TextWrapMode::Wrap),
                    (width - text_offset).max(0.0),
                    FontSelection::Style(subheadline()),
                );
            let lines = state.size().y + level.size().y + GRID / 2.0 + note.size().y;
            let (top, card) = ui.allocate_exact_size(vec2(width, GLYPH.max(lines)), Sense::hover());
            // One element: a group named by the sentence, with where it comes from as
            // its description.
            card.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, hero.sentence()));
            ui.ctx().accesskit_node_builder(card.id, |node| {
                node.set_role(Role::Group);
                node.set_description(hero.note.clone());
            });

            // The glyph, its wisps easing to the level. While it diffuses the unlit
            // wisps are faint tracks, a scale of three; off, they are left out, as faint
            // tracks would read as muted.
            let wisps =
                ui.ctx()
                    .animate_value_with_time(id.with("wisps"), hero.wisps(), MOTION_SECS);
            let ink = if hero.diffusing() {
                Ink {
                    body: palette.label,
                    mist: palette.accent,
                    ground: palette.grouped,
                    track: Some(Palette::tint(palette.label, 12)),
                }
            } else {
                Ink {
                    body: palette.gray,
                    mist: palette.gray,
                    ground: palette.grouped,
                    track: None,
                }
            };
            // Placed by its ink, not its square: the ink lines up with the card's other
            // content, and the words follow it.
            let glyph = Rect::from_center_size(
                pos2(top.left() - ink_left * scale + GLYPH / 2.0, top.center().y),
                vec2(GLYPH, GLYPH),
            );
            match hero.model {
                Some(Model::Hvac2) => glyphs::hvac2(ui.painter(), glyph, &ink, wisps),
                _ => glyphs::air2(ui.painter(), glyph, &ink, wisps),
            }

            // The words, centered on the glyph, and the switch at the trailing edge,
            // centered on the state's line.
            let text_left = top.left() + text_offset;
            let mut y = top.center().y - lines / 2.0;
            let state_middle = y + state.size().y / 2.0;
            for (galley, after) in [(state, 0.0), (level, GRID / 2.0), (note, 0.0)] {
                let height = galley.size().y;
                ui.painter()
                    .galley(pos2(text_left, y), galley, palette.label);
                y += height + after;
            }
            let at = Rect::from_min_size(
                pos2(
                    top.right() - switch_size.x,
                    state_middle - switch_size.y / 2.0,
                ),
                switch_size,
            );
            // Laid over the top row, which is taken already: a child that doesn't move
            // the card's cursor.
            let mut on = hero.power.unwrap_or(false);
            let mut over = ui.new_child(UiBuilder::new().max_rect(at));
            if switch(&mut over, &mut on, "Power").changed() {
                changed.power = Some(on);
            }

            ui.add_space(2.0 * GRID);
            let mut shown = hero.level;
            if let Some(Some(level)) =
                segmented_wide(ui, &mut shown, &LEVELS, "Intensity", SEGMENTS_HEIGHT).clicked
            {
                changed.level = Some(level);
            }
        });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hero(power: Option<bool>, level: Option<u8>) -> HeroState {
        HeroState {
            name: "Bedroom".to_owned(),
            power,
            level,
            ..HeroState::default()
        }
    }

    #[test]
    fn the_hero_says_what_it_is_doing_in_words() {
        let on = hero(Some(true), Some(2));
        assert_eq!(on.state_words(), "Diffusing");
        assert_eq!(on.intensity_words(), "Medium intensity");
        assert_eq!(on.sentence(), "Bedroom, diffusing at Medium intensity");
        let off = hero(Some(false), None);
        assert_eq!(off.state_words(), "Off");
        assert_eq!(off.intensity_words(), "Not diffusing");
        assert_eq!(off.sentence(), "Bedroom, off");
        let unknown = hero(None, None);
        assert_eq!(unknown.state_words(), "Not reported yet");
        assert_eq!(unknown.sentence(), "Bedroom, not reported yet");
        assert_eq!(unknown.intensity_words(), "Intensity not reported");
    }

    #[test]
    fn the_state_is_grey_when_old_or_unknown_and_orange_while_sending() {
        for dark in [false, true] {
            let palette = Palette::for_dark(dark);
            let mut state = hero(Some(true), Some(1));
            assert_eq!(state.state_color(&palette), palette.label);
            state.stale = true;
            assert_eq!(state.state_color(&palette), palette.secondary_label);
            assert_eq!(
                hero(None, None).state_color(&palette),
                palette.secondary_label
            );
            state.sending = true;
            assert_eq!(state.state_words(), "Sending…");
            assert_eq!(state.state_color(&palette), palette.orange);
            assert_eq!(
                state.sentence(),
                "Bedroom, sending: diffusing at Low intensity"
            );
            // What was sent and not yet reported on is said in orange under it.
            state.pending = true;
            assert_eq!(state.note_color(&palette), palette.orange);
        }
    }

    #[test]
    fn the_wisps_are_the_level() {
        assert_eq!(hero(Some(true), Some(3)).wisps(), 3.0);
        assert_eq!(hero(Some(false), Some(1)).wisps(), 1.0);
        assert_eq!(hero(Some(true), None).wisps(), 0.0);
        assert_eq!(hero(Some(true), Some(9)).wisps(), 0.0);
    }
}
