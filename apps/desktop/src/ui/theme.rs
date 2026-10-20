//! The look, as Apple's macOS guidelines have it (see the spec section "Design
//! language"): the system colors for light and dark, the macOS text styles in SF Pro,
//! and the spacing.
//!
//! The fonts are the Mac's own, read from `/System/Library/Fonts` when the app starts.
//! They are never copied into the repo or the bundle; without them the app falls back
//! to egui's fonts.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use eframe::egui::style::{Selection, WidgetVisuals, Widgets};
use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, FontTweak, Margin,
    Shadow, Stroke, TextStyle, Theme, ThemePreference, Visuals, vec2,
};
use eframe::epaint::text::VariationCoords;
use scent_core::Online;

use crate::state::BrokerState;

/// Where macOS keeps its system fonts.
pub const SYSTEM_FONTS: &str = "/System/Library/Fonts";

/// SF Pro, a variable font, and SF Mono.
const SF_PRO_FILE: &str = "SFNS.ttf";
const SF_MONO_FILE: &str = "SFNSMono.ttf";

/// The names the fonts go by in egui.
const SF_PRO: &str = "SF Pro";
const SF_PRO_SEMIBOLD: &str = "SF Pro Semibold";
const SF_MONO: &str = "SF Mono";

/// The family for semibold text: SF Pro at weight 590, or egui's font without it.
const SEMIBOLD_FAMILY: &str = "Semibold";

/// The regular and semibold weights, on the fonts' `wght` axis. SF Mono's own default
/// is lighter than regular, so it is set too.
const REGULAR_WEIGHT: f32 = 400.0;
const SEMIBOLD_WEIGHT: f32 = 590.0;

/// SF Pro's optical size for text below 20 pt, on its `opsz` axis: the smallest it has,
/// which is what macOS draws such text with. The font's own default is a display size,
/// too tight for 11 and 13 pt.
const TEXT_OPTICAL_SIZE: f32 = 17.0;

/// The name of the Headline text style.
const HEADLINE: &str = "Headline";

/// The main window's size when it first opens, and the least it can be resized to.
pub const WINDOW_SIZE: [f32; 2] = [1100.0, 720.0];
pub const MIN_WINDOW_SIZE: [f32; 2] = [900.0, 600.0];

/// The layout grid. Spacing is a multiple of it.
pub const GRID: f32 = 8.0;
/// The margin around the content of the sidebar, the detail pane and the settings.
pub const MARGIN: f32 = 20.0;
/// The toolbar along the top of the detail pane: as tall as the title bar, so that its
/// controls are centered on the traffic lights' line. With a full-size content view the
/// traffic lights stay where AppKit puts them in a standard 28 pt title bar (winit only
/// hides or disables them, never moves them), centered about 14 pt from the top.
pub const TOOLBAR_HEIGHT: f32 = 28.0;
/// The room at the top of the sidebar for the traffic lights, which sit over it.
pub const SIDEBAR_TOP_INSET: f32 = 40.0;
/// The width of the sidebar when the window first opens, and how narrow and wide the
/// person can drag it.
pub const SIDEBAR_WIDTH: f32 = 260.0;
pub const SIDEBAR_MIN_WIDTH: f32 = 200.0;
pub const SIDEBAR_MAX_WIDTH: f32 = 360.0;

/// Corner radii: grouped boxes, notices, and controls (push buttons, the sidebar's
/// selection, segmented controls).
pub const BOX_RADIUS: u8 = 10;
pub const NOTICE_RADIUS: u8 = 8;
pub const CONTROL_RADIUS: u8 = 6;

/// How long the motion that a person's action causes takes: the switch, the
/// segmented control's selection, and the log pane opening.
pub const MOTION_SECS: f32 = 0.2;

/// Apple's system colors for one appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// systemBlue: the selection, default buttons, switches that are on.
    pub accent: Color32,
    /// systemGreen: answering.
    pub green: Color32,
    /// systemOrange: waiting, and values sent but not yet reported.
    pub orange: Color32,
    /// systemRed: errors, a refused login.
    pub red: Color32,
    /// systemGray: unknown, silent.
    pub gray: Color32,
    pub label: Color32,
    /// Ages, sources, stale readings, sidebar section headings.
    pub secondary_label: Color32,
    pub separator: Color32,
    pub window: Color32,
    /// Approximates the sidebar's material.
    pub sidebar: Color32,
    /// The inside of a grouped box.
    pub grouped: Color32,
}

/// An opacity given in percent, as an alpha value.
const fn alpha(percent: u32) -> u8 {
    ((percent * 255 + 50) / 100) as u8
}

/// White at `alpha` opacity (premultiplied, as egui keeps colors).
const fn white(alpha: u8) -> Color32 {
    Color32::from_rgba_premultiplied(alpha, alpha, alpha, alpha)
}

const fn hex(rgb: u32) -> Color32 {
    let [_, r, g, b] = rgb.to_be_bytes();
    Color32::from_rgb(r, g, b)
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        accent: hex(0x007AFF),
        green: hex(0x34C759),
        orange: hex(0xFF9500),
        red: hex(0xFF3B30),
        gray: hex(0x8E8E93),
        label: Color32::from_black_alpha(alpha(85)),
        secondary_label: Color32::from_black_alpha(alpha(50)),
        separator: Color32::from_black_alpha(alpha(10)),
        window: hex(0xECECEC),
        sidebar: hex(0xE3E3E6),
        grouped: hex(0xFFFFFF),
    };

    pub const DARK: Palette = Palette {
        accent: hex(0x0A84FF),
        green: hex(0x30D158),
        orange: hex(0xFF9F0A),
        red: hex(0xFF453A),
        gray: hex(0x98989D),
        label: white(alpha(85)),
        secondary_label: white(alpha(55)),
        separator: white(alpha(10)),
        window: hex(0x323232),
        sidebar: hex(0x2B2B2D),
        grouped: hex(0x2C2C2E),
    };

    pub fn for_dark(dark: bool) -> Palette {
        if dark { Palette::DARK } else { Palette::LIGHT }
    }

    /// The palette of the appearance `ui` is drawn in.
    pub fn of(ui: &egui::Ui) -> Palette {
        Palette::for_dark(ui.visuals().dark_mode)
    }

    /// `color` at `percent` opacity, to lay over a background: a notice's tint.
    pub fn tint(color: Color32, percent: u32) -> Color32 {
        color.gamma_multiply(f32::from(alpha(percent)) / 255.0)
    }
}

/// The color of a status dot for whether a diffuser answers: green when it does,
/// orange while it is waited for, gray for the rest (not pinged yet, or silent).
pub fn status_color(palette: &Palette, online: Online) -> Color32 {
    match online {
        Online::Online => palette.green,
        Online::Waiting => palette.orange,
        Online::Offline | Online::Unknown => palette.gray,
    }
}

/// The color of the broker's status dot: green when connected, orange while connecting
/// or after a disconnection, red when the login was refused.
pub fn broker_color(palette: &Palette, broker: &BrokerState) -> Color32 {
    match broker {
        BrokerState::Connected => palette.green,
        BrokerState::Connecting | BrokerState::Disconnected(_) => palette.orange,
        BrokerState::LoginRefused(_) => palette.red,
    }
}

/// Title 1, 22 pt: the open diffuser's name.
pub fn title1() -> TextStyle {
    TextStyle::Heading
}

/// Headline, 13 pt semibold: section headings.
pub fn headline() -> TextStyle {
    TextStyle::Name(HEADLINE.into())
}

/// Subheadline, 11 pt: ages, sources and sidebar section headings, in the secondary
/// label color.
pub fn subheadline() -> TextStyle {
    TextStyle::Small
}

/// The family of semibold text.
pub fn semibold() -> FontFamily {
    FontFamily::Name(SEMIBOLD_FAMILY.into())
}

/// The macOS text styles. Body and Button are 13 pt; Monospace is SF Mono 11 pt, for
/// the log.
pub fn text_styles() -> BTreeMap<TextStyle, FontId> {
    BTreeMap::from([
        (title1(), FontId::proportional(22.0)),
        (headline(), FontId::new(13.0, semibold())),
        (TextStyle::Body, FontId::proportional(13.0)),
        (TextStyle::Button, FontId::proportional(13.0)),
        (subheadline(), FontId::proportional(11.0)),
        (TextStyle::Monospace, FontId::monospace(11.0)),
    ])
}

/// A font instance: the weight, and the optical size if the font has one to set.
fn instance(weight: f32, optical_size: Option<f32>) -> FontTweak {
    let mut coords = VariationCoords::new([(b"wght", weight)]);
    if let Some(size) = optical_size {
        coords.push(b"opsz", size);
    }
    FontTweak {
        coords,
        ..FontTweak::default()
    }
}

/// SF Pro twice over, from the bytes of `SFNS.ttf`: the regular instance and the
/// semibold one, both at the optical size for text. They share the one copy of the
/// bytes.
fn sf_pro(bytes: &'static [u8]) -> [(&'static str, FontData); 2] {
    let at = |weight| instance(weight, Some(TEXT_OPTICAL_SIZE));
    [
        (
            SF_PRO,
            FontData::from_static(bytes).tweak(at(REGULAR_WEIGHT)),
        ),
        (
            SF_PRO_SEMIBOLD,
            FontData::from_static(bytes).tweak(at(SEMIBOLD_WEIGHT)),
        ),
    ]
}

/// SF Mono at its regular weight, from the bytes of `SFNSMono.ttf`.
fn sf_mono(bytes: &'static [u8]) -> FontData {
    FontData::from_static(bytes).tweak(instance(REGULAR_WEIGHT, None))
}

/// The bytes of the font file at `path`, read once and then kept for as long as the app
/// runs, as the fonts made from them are. Two egui fonts can then share one copy (SF
/// Pro is 8 MB), and installing the theme again reads nothing. `None` when the file
/// can't be read.
fn font_file(path: &Path) -> Option<&'static [u8]> {
    static READ: OnceLock<Mutex<HashMap<PathBuf, &'static [u8]>>> = OnceLock::new();
    let mut read = READ
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(bytes) = read.get(path) {
        return Some(bytes);
    }
    let bytes: &'static [u8] = Vec::leak(std::fs::read(path).ok()?);
    read.insert(path.to_owned(), bytes);
    Some(bytes)
}

/// egui's fonts with SF Pro and SF Mono from `fonts_dir` put first, and egui's own
/// kept behind them for the characters SF lacks. A font that isn't there is left out,
/// and its families keep egui's fonts alone.
pub fn font_definitions(fonts_dir: &Path) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let mut semibold_fonts = fonts.families[&FontFamily::Proportional].clone();
    if let Some(bytes) = font_file(&fonts_dir.join(SF_PRO_FILE)) {
        let [(regular_name, regular), (semibold_name, semibold)] = sf_pro(bytes);
        fonts
            .font_data
            .insert(regular_name.to_owned(), Arc::new(regular));
        fonts
            .font_data
            .insert(semibold_name.to_owned(), Arc::new(semibold));
        put_first(&mut fonts, FontFamily::Proportional, regular_name);
        semibold_fonts.insert(0, semibold_name.to_owned());
    }
    fonts.families.insert(semibold(), semibold_fonts);
    if let Some(bytes) = font_file(&fonts_dir.join(SF_MONO_FILE)) {
        fonts
            .font_data
            .insert(SF_MONO.to_owned(), Arc::new(sf_mono(bytes)));
        put_first(&mut fonts, FontFamily::Monospace, SF_MONO);
    }
    fonts
}

fn put_first(fonts: &mut FontDefinitions, family: FontFamily, name: &str) {
    fonts
        .families
        .entry(family)
        .or_default()
        .insert(0, name.to_owned());
}

/// Sets the app's look on `ctx`: it follows the system's light or dark setting, with
/// the fonts from `fonts_dir` (see [`font_definitions`]), the text styles, and the
/// colors and spacing of each appearance. The fonts take effect from the next pass.
pub fn install(ctx: &egui::Context, fonts_dir: &Path) {
    ctx.set_fonts(font_definitions(fonts_dir));
    ctx.set_theme(ThemePreference::System);
    for theme in [Theme::Light, Theme::Dark] {
        ctx.style_mut_of(theme, |style| {
            let dark = theme == Theme::Dark;
            style.text_styles = text_styles();
            style.visuals = visuals(&Palette::for_dark(dark), dark, &style.visuals);
            let spacing = &mut style.spacing;
            spacing.item_spacing = vec2(GRID, GRID);
            spacing.button_padding = vec2(12.0, 3.0);
            spacing.interact_size = vec2(40.0, 22.0);
            spacing.window_margin = Margin::same(MARGIN as i8);
            spacing.menu_margin = Margin::same(6);
            spacing.text_edit_width = 280.0;
            style.interaction.selectable_labels = false;
            style.animation_time = MOTION_SECS;
        });
    }
}

/// The visuals of one appearance, starting from what egui had (`was`) so that settings
/// outside the palette, such as a test's cursor blink, are kept.
fn visuals(palette: &Palette, dark: bool, was: &Visuals) -> Visuals {
    let control = |fill: Color32, stroke: Stroke| WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: stroke,
        corner_radius: CornerRadius::same(CONTROL_RADIUS),
        fg_stroke: Stroke::new(1.0, palette.label),
        expansion: 0.0,
    };
    // A push button is white with a darker hairline edge in light mode, and a lighter
    // gray with a lighter one in dark mode; pressed, it darkens (light) or brightens
    // (dark).
    let (button, pressed, edge) = if dark {
        (white(46), white(70), Stroke::new(0.5, white(31)))
    } else {
        (
            Color32::WHITE,
            hex(0xE5E5E5),
            Stroke::new(0.5, Color32::from_black_alpha(38)),
        )
    };
    // Hairlines: one pixel on a Retina screen.
    let separator = Stroke::new(0.5, palette.separator);
    Visuals {
        dark_mode: dark,
        override_text_color: None,
        weak_text_color: Some(palette.secondary_label),
        widgets: Widgets {
            noninteractive: control(palette.window, separator),
            inactive: control(button, edge),
            hovered: control(button, edge),
            active: control(pressed, edge),
            open: control(pressed, edge),
        },
        selection: Selection {
            bg_fill: Palette::tint(palette.accent, 30),
            stroke: Stroke::new(1.0, palette.accent),
        },
        hyperlink_color: palette.accent,
        faint_bg_color: Palette::tint(palette.label, 4),
        extreme_bg_color: palette.grouped,
        // A text field is white in light mode and a little lighter than its box in dark
        // mode, with the controls' hairline edge.
        text_edit_bg_color: Some(if dark { white(15) } else { palette.grouped }),
        code_bg_color: palette.grouped,
        warn_fg_color: palette.orange,
        error_fg_color: palette.red,
        window_corner_radius: CornerRadius::same(BOX_RADIUS),
        window_shadow: Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(if dark { 110 } else { 50 }),
        },
        window_fill: palette.window,
        window_stroke: separator,
        menu_corner_radius: CornerRadius::same(CONTROL_RADIUS),
        panel_fill: palette.window,
        popup_shadow: Shadow {
            offset: [0, 4],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(if dark { 90 } else { 40 }),
        },
        interact_cursor: None,
        striped: false,
        ..was.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use eframe::egui::{Color32, FontData, FontDefinitions, FontFamily};
    use eframe::epaint::text::Tag;
    use scent_core::Online;

    use super::*;

    fn rgb(hex: u32) -> Color32 {
        let [_, r, g, b] = hex.to_be_bytes();
        Color32::from_rgb(r, g, b)
    }

    /// The color as the spec writes it: the color and its opacity, not premultiplied.
    fn rgba(color: Color32) -> [u8; 4] {
        color.to_srgba_unmultiplied()
    }

    #[test]
    fn palette_matches_the_spec() {
        let light = Palette::for_dark(false);
        assert_eq!(light.accent, rgb(0x007AFF));
        assert_eq!(light.green, rgb(0x34C759));
        assert_eq!(light.orange, rgb(0xFF9500));
        assert_eq!(light.red, rgb(0xFF3B30));
        assert_eq!(light.gray, rgb(0x8E8E93));
        assert_eq!(light.window, rgb(0xECECEC));
        assert_eq!(light.sidebar, rgb(0xE3E3E6));
        assert_eq!(light.grouped, rgb(0xFFFFFF));
        // Black at 85%, 50% and 10%.
        assert_eq!(rgba(light.label), [0, 0, 0, 217]);
        assert_eq!(rgba(light.secondary_label), [0, 0, 0, 128]);
        assert_eq!(rgba(light.separator), [0, 0, 0, 26]);

        let dark = Palette::for_dark(true);
        assert_eq!(dark.accent, rgb(0x0A84FF));
        assert_eq!(dark.green, rgb(0x30D158));
        assert_eq!(dark.orange, rgb(0xFF9F0A));
        assert_eq!(dark.red, rgb(0xFF453A));
        assert_eq!(dark.gray, rgb(0x98989D));
        assert_eq!(dark.window, rgb(0x323232));
        assert_eq!(dark.sidebar, rgb(0x2B2B2D));
        assert_eq!(dark.grouped, rgb(0x2C2C2E));
        // White at 85%, 55% and 10%.
        assert_eq!(rgba(dark.label), [255, 255, 255, 217]);
        assert_eq!(rgba(dark.secondary_label), [255, 255, 255, 140]);
        assert_eq!(rgba(dark.separator), [255, 255, 255, 26]);
    }

    #[test]
    fn status_colors_follow_online() {
        for dark in [false, true] {
            let palette = Palette::for_dark(dark);
            assert_eq!(status_color(&palette, Online::Online), palette.green);
            assert_eq!(status_color(&palette, Online::Waiting), palette.orange);
            assert_eq!(status_color(&palette, Online::Unknown), palette.gray);
            assert_eq!(status_color(&palette, Online::Offline), palette.gray);
        }
    }

    #[test]
    fn broker_colors_follow_the_connection() {
        let palette = Palette::for_dark(false);
        let color = |broker| broker_color(&palette, &broker);
        assert_eq!(color(BrokerState::Connected), palette.green);
        assert_eq!(color(BrokerState::Connecting), palette.orange);
        assert_eq!(
            color(BrokerState::Disconnected("Gone.".into())),
            palette.orange
        );
        assert_eq!(color(BrokerState::LoginRefused("No.".into())), palette.red);
    }

    /// A folder that exists and holds no fonts. Removed when dropped.
    struct EmptyDir(PathBuf);

    impl EmptyDir {
        fn new(name: &str) -> EmptyDir {
            let dir = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            EmptyDir(dir)
        }
    }

    impl Drop for EmptyDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn fonts_fall_back_when_missing() {
        let dir = EmptyDir::new("scent-no-fonts");
        let fonts = font_definitions(&dir.0);
        let defaults = FontDefinitions::default();
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            assert_eq!(fonts.families[&family], defaults.families[&family]);
        }
        // The semibold family is still there, so the headline style has fonts to use.
        assert_eq!(
            fonts.families[&semibold()],
            defaults.families[&FontFamily::Proportional]
        );
        assert!(
            fonts.font_data.keys().all(|name| !name.starts_with("SF")),
            "{:?}",
            fonts.font_data.keys()
        );
    }

    /// The value `font` is set to on `axis`, if it is set: the last one given counts.
    fn coord(font: &FontData, axis: &[u8; 4]) -> Option<f32> {
        let axis = Tag::new(axis);
        let coords = font.tweak.coords.as_ref();
        coords
            .iter()
            .rev()
            .find(|(tag, _)| *tag == axis)
            .map(|&(_, value)| value)
    }

    #[test]
    fn semibold_uses_weight_590() {
        // The bytes are not looked at until a font is drawn with, so any will do here.
        let [(regular_name, regular), (semibold_name, semibold)] = sf_pro(&[0; 4]);
        assert_eq!(coord(&regular, b"wght"), Some(400.0));
        assert_eq!(coord(&semibold, b"wght"), Some(590.0));
        assert_ne!(regular_name, semibold_name);
        // Both at the optical size for text; SF Mono at its regular weight.
        assert_eq!(coord(&regular, b"opsz"), Some(17.0));
        assert_eq!(coord(&semibold, b"opsz"), Some(17.0));
        // The two SF Pro instances are one copy of the font.
        assert!(std::ptr::eq(regular.font.as_ptr(), semibold.font.as_ptr()));
        let mono = sf_mono(&[0; 4]);
        assert_eq!(coord(&mono, b"wght"), Some(400.0));
        assert_eq!(coord(&mono, b"opsz"), None);
    }

    #[test]
    fn a_font_file_is_read_once() {
        let dir = EmptyDir::new("scent-one-font");
        let path = dir.0.join("Font.ttf");
        std::fs::write(&path, b"font").unwrap();
        let first = font_file(&path).unwrap();
        std::fs::write(&path, b"changed").unwrap();
        let again = font_file(&path).unwrap();
        assert!(std::ptr::eq(first, again));
        assert_eq!(again, b"font");
        assert!(font_file(&dir.0.join("Missing.ttf")).is_none());
    }

    #[test]
    fn the_text_styles_are_the_macos_ones() {
        let styles = text_styles();
        let size = |style: &TextStyle| styles[style].size;
        assert_eq!(size(&title1()), 22.0);
        assert_eq!(size(&headline()), 13.0);
        assert_eq!(styles[&headline()].family, semibold());
        assert_eq!(size(&TextStyle::Body), 13.0);
        assert_eq!(size(&TextStyle::Button), 13.0);
        assert_eq!(size(&subheadline()), 11.0);
        assert_eq!(size(&TextStyle::Monospace), 11.0);
        assert_eq!(styles[&TextStyle::Monospace].family, FontFamily::Monospace);
    }
}
