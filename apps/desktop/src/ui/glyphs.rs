//! The app's drawn glyphs, since egui can't use SF Symbols: the Air 2 (a capsule) and
//! the HVAC 2 (a low box with vent slits), each with up to three wisps of mist rising from
//! its top for its level; signal bars; the log's disclosure triangle and its marks for
//! sent (↗), heard (↙) and failed (!); the large diffuser of an empty state; and the
//! rounded tile they sit on in the sidebar.
//!
//! Each is drawn on a grid of 28 units, the size of a sidebar tile, scaled to the square
//! it is given: strokes are 1.5 units (1.5 pt on a tile, scaled with the glyph, never
//! thinner than 1.25 pt), with round ends and joins, and corners rounded as the tiles'
//! are. A diffuser is centered: with mist or its tracks, the body is in the lower half and
//! the mist above it; with neither, the body alone sits in the middle of its square. They
//! are painted, not laid out: they have no accessibility node of their own, since the
//! words beside them say what they show.

use std::f32::consts::TAU;

use eframe::egui::{
    Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2,
};
use eframe::epaint::PathShape;

/// The grid the glyphs are drawn on, and the stroke on it.
pub const UNIT: f32 = 28.0;
pub(super) const STROKE: f32 = 1.5;
/// The thinnest a stroke gets, at small sizes, so that it stays crisp.
const MIN_STROKE: f32 = 1.25;

/// A tile's corner radius, as a share of its side: 6 pt on a 28 pt tile, the radius of
/// the app's controls.
const TILE_CORNER: f32 = 6.0 / UNIT;

/// The wisps of mist: where the middle one starts (its foot) and how tall it is, how far
/// each sways either side of its line (one gentle S), and the lines of the three, 3.5
/// units apart so that they stay apart at 1x. The outer two start a unit higher and lean
/// out, so that the plume fans.
const WISP_FOOT: f32 = 11.5;
const WISP_HEIGHT: f32 = 7.0;
const WISP_SWAY: f32 = 0.7;
const WISP_LINES: [f32; 3] = [10.5, 14.0, 17.5];
const OUTER_LIFT: f32 = 1.0;
const OUTER_LEAN: f32 = 0.75;

/// Where a diffuser glyph's ink runs across the grid, its body and mist together, for a
/// layout that lines the ink up rather than the square.
pub const INK_X: [f32; 2] = [8.0, 20.0];

/// Where each body runs down the grid, top to bottom, under its mist.
pub(super) const AIR2_BODY: [f32; 2] = [14.0, 24.0];
const HVAC2_BODY: [f32; 2] = [15.5, 23.5];

/// Where the Air 2's capsule runs across the grid, 6 units wide, and its mist outlet: a
/// short slit from one end to the other.
pub(super) const AIR2_SIDES: [f32; 2] = [11.0, 17.0];
pub(super) const AIR2_OUTLET: [[f32; 2]; 2] = [[13.0, 16.75], [15.0, 16.75]];

/// The colors a diffuser glyph is drawn in.
#[derive(Debug, Clone, Copy)]
pub struct Ink {
    /// The body.
    pub body: Color32,
    /// The wisps of mist.
    pub mist: Color32,
    /// What the glyph is drawn on. The glyph's colors are made solid over it, so that
    /// where a stroke's round end overlaps it nothing shows darker.
    pub ground: Color32,
    /// The wisps past the level, as faint tracks, as signal bars show the bars not
    /// filled; `None` leaves them out. The hero shows them while it diffuses, as a scale
    /// of three.
    pub track: Option<Color32>,
}

/// The 28-unit grid laid on a square, centered in a rect.
struct Grid {
    origin: Pos2,
    scale: f32,
}

impl Grid {
    fn of(rect: Rect) -> Grid {
        let side = rect.width().min(rect.height());
        Grid {
            origin: rect.center() - vec2(side, side) / 2.0,
            scale: side / UNIT,
        }
    }

    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.scale
    }

    /// The grid for a body that runs `body` down it (top and bottom), drawn with `ink`
    /// at `level`. With mist or tracks above it, the body keeps its place in the lower
    /// half. With neither, it alone is the glyph, so it moves up to the middle of the
    /// square; while the first wisp eases in or out, it moves part of the way.
    fn for_body(rect: Rect, body: [f32; 2], ink: &Ink, level: f32) -> Grid {
        let grid = Grid::of(rect);
        if ink.track.is_some() {
            return grid;
        }
        let alone = 1.0 - level.clamp(0.0, 1.0);
        let up = ((body[0] + body[1]) / 2.0 - UNIT / 2.0) * alone;
        Grid {
            origin: grid.origin - vec2(0.0, grid.len(up)),
            scale: grid.scale,
        }
    }

    fn len(&self, units: f32) -> f32 {
        units * self.scale
    }

    fn rect(&self, left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect::from_min_max(self.at(left, top), self.at(right, bottom))
    }

    fn stroke(&self) -> f32 {
        (STROKE * self.scale).max(MIN_STROKE)
    }
}

/// `color` laid over the opaque `ground`: solid, so that strokes that overlap don't
/// darken where they meet.
pub fn solid(color: Color32, ground: Color32) -> Color32 {
    let keep = 1.0 - f32::from(color.a()) / 255.0;
    let mix = |top: u8, base: u8| (f32::from(top) + f32::from(base) * keep).round() as u8;
    Color32::from_rgb(
        mix(color.r(), ground.r()),
        mix(color.g(), ground.g()),
        mix(color.b(), ground.b()),
    )
}

/// A line through `points`, `width` wide, with round ends. A line of three points or
/// fewer has corners, and they are rounded too; a longer one is a curve, smooth already.
fn round_line(painter: &Painter, points: Vec<Pos2>, width: f32, color: Color32) {
    let radius = width / 2.0;
    let rounded: Vec<Pos2> = if points.len() <= 3 {
        points.clone()
    } else {
        [points.first(), points.last()]
            .into_iter()
            .flatten()
            .copied()
            .collect()
    };
    for point in rounded {
        painter.circle_filled(point, radius, color);
    }
    painter.add(Shape::Path(PathShape::line(
        points,
        Stroke::new(width, color),
    )));
}

/// How much of each wisp shows at `level` (0 to 3, between whole numbers while it
/// eases): the left, middle and right one. Low is the middle wisp, Medium the outer
/// pair, High all three.
fn wisp_shares(level: f32) -> [f32; 3] {
    let level = level.clamp(0.0, 3.0);
    let outer = (level - 1.0).clamp(0.0, 1.0);
    let middle = if level <= 1.0 {
        level
    } else if level <= 2.0 {
        2.0 - level
    } else {
        level - 2.0
    };
    [outer, middle, outer]
}

/// The line of wisp `i` (the left, middle or right one) on the grid, from its foot up,
/// `length` of the way (1 for all of it).
pub(super) fn wisp_line(i: usize, length: f32) -> Vec<Pos2> {
    // The left one sways and leans the other way from the right one: the plume is
    // symmetric.
    let (side, foot) = match i {
        0 => (-1.0, WISP_FOOT - OUTER_LIFT),
        1 => (1.0, WISP_FOOT),
        _ => (1.0, WISP_FOOT - OUTER_LIFT),
    };
    let lean = if i == 1 { 0.0 } else { OUTER_LEAN * side };
    let line = WISP_LINES[i];
    let steps = 20;
    (0..=steps)
        .map(|step| {
            let along = length * step as f32 / steps as f32;
            let x = line + side * WISP_SWAY * (TAU * along).sin() + lean * along;
            pos2(x, foot - WISP_HEIGHT * along)
        })
        .collect()
}

/// The wisps of mist over a body, for `level`. A wisp part-way in is drawn part of the
/// way from its track (or the ground) to its own color, and, with no track, a little
/// shorter.
fn wisps(painter: &Painter, grid: &Grid, ink: &Ink, level: f32) {
    let lit = solid(ink.mist, ink.ground);
    let track = ink.track.map(|track| solid(track, ink.ground));
    for (i, shown) in wisp_shares(level).into_iter().enumerate() {
        let (from, length) = match track {
            Some(track) => (track, 1.0),
            None if shown <= 0.0 => continue,
            None => (ink.ground, 0.7 + 0.3 * shown),
        };
        let points = wisp_line(i, length)
            .into_iter()
            .map(|point| grid.at(point.x, point.y))
            .collect();
        round_line(
            painter,
            points,
            grid.stroke(),
            from.lerp_to_gamma(lit, shown),
        );
    }
}

/// The Air 2: an upright capsule in the lower half with its mist outlet, a short slit,
/// near the top, and `level` wisps (0 to 3) rising over it. With no wisps and no tracks,
/// the capsule sits in the middle.
pub fn air2(painter: &Painter, rect: Rect, ink: &Ink, level: f32) {
    let grid = Grid::for_body(rect, AIR2_BODY, ink, level);
    let body = solid(ink.body, ink.ground);
    // 6 by 10 units: a capsule, so its radius is half its width.
    let [top, bottom] = AIR2_BODY;
    let [left, right] = AIR2_SIDES;
    let capsule = grid.rect(left, top, right, bottom);
    painter.rect_stroke(
        capsule,
        CornerRadius::from(grid.len((right - left) / 2.0)),
        Stroke::new(grid.stroke(), body),
        StrokeKind::Middle,
    );
    // The mist outlet: a short slit across the top, under the wisps.
    let outlet = AIR2_OUTLET.map(|[x, y]| grid.at(x, y)).to_vec();
    round_line(painter, outlet, grid.stroke(), body);
    wisps(painter, &grid, ink, level);
}

/// The HVAC 2: a low box in the lower half with two vent slits, and `level` wisps (0 to 3)
/// rising over it. With no wisps and no tracks, the box sits in the middle.
pub fn hvac2(painter: &Painter, rect: Rect, ink: &Ink, level: f32) {
    let grid = Grid::for_body(rect, HVAC2_BODY, ink, level);
    let body = solid(ink.body, ink.ground);
    // 12 by 8 units, with the tiles' corner language.
    let [top, bottom] = HVAC2_BODY;
    let case = grid.rect(8.0, top, 20.0, bottom);
    painter.rect_stroke(
        case,
        CornerRadius::from(grid.len(2.0)),
        Stroke::new(grid.stroke(), body),
        StrokeKind::Middle,
    );
    for y in [18.0, 21.0] {
        let slit = vec![grid.at(11.0, y), grid.at(17.0, y)];
        round_line(painter, slit, grid.stroke(), body);
    }
    wisps(painter, &grid, ink, level);
}

/// The large diffuser of an empty state: the Air 2 with its three wisps faint, waiting.
pub fn empty_diffuser(painter: &Painter, rect: Rect, color: Color32, ground: Color32) {
    let faint = solid(color, ground).lerp_to_gamma(ground, 0.6);
    let ink = Ink {
        body: color,
        mist: faint,
        ground,
        track: None,
    };
    air2(painter, rect, &ink, 3.0);
}

/// A rounded tile, filled, for a glyph to sit on.
pub fn tile(painter: &Painter, rect: Rect, fill: Color32) {
    let radius = rect.width().min(rect.height()) * TILE_CORNER;
    painter.rect_filled(rect, CornerRadius::from(radius), fill);
}

/// Four signal bars, bottom-aligned in `rect`, rising left to right: the first `filled`
/// in `on`, the rest in `off`.
pub fn signal_bars(painter: &Painter, rect: Rect, on: Color32, off: Color32, filled: u8) {
    let bars = 4;
    // Four bars and three gaps half as wide.
    let width = rect.width() / (bars as f32 + (bars - 1) as f32 * 0.5);
    let gap = width / 2.0;
    let radius = CornerRadius::from(width * 0.3);
    for bar in 0..bars {
        let height = rect.height() * (0.4 + 0.2 * bar as f32);
        let left = rect.left() + bar as f32 * (width + gap);
        let shape = Rect::from_min_max(
            pos2(left, rect.bottom() - height),
            pos2(left + width, rect.bottom()),
        );
        let color = if bar < filled { on } else { off };
        painter.rect_filled(shape, radius, color);
    }
}

/// A disclosure triangle, ▸ when closed and ▾ when open, filled, its corners softened.
pub fn disclosure(painter: &Painter, rect: Rect, color: Color32, open: bool) {
    let grid = Grid::of(rect);
    let points = if open {
        [(8.0, 11.0), (20.0, 11.0), (14.0, 19.0)]
    } else {
        [(11.0, 8.0), (19.0, 14.0), (11.0, 20.0)]
    };
    let points: Vec<Pos2> = points.into_iter().map(|(x, y)| grid.at(x, y)).collect();
    let soft = Stroke::new(grid.len(1.5), color);
    painter.add(Shape::Path(PathShape::convex_polygon(points, color, soft)));
}

/// What a log row's mark says happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Sent to the diffuser: ↗
    Sent,
    /// Heard from it, or from the broker: ↙
    Heard,
    /// Failed: !
    Failed,
}

/// A log row's mark: an arrow out (↗) or in (↙) with its head, or "!".
pub fn mark(painter: &Painter, rect: Rect, color: Color32, mark: Mark) {
    let grid = Grid::of(rect);
    let width = grid.stroke();
    let line = |points: &[(f32, f32)]| {
        let points = points.iter().map(|&(x, y)| grid.at(x, y)).collect();
        round_line(painter, points, width, color);
    };
    match mark {
        Mark::Sent => {
            line(&[(8.5, 19.5), (19.0, 9.0)]);
            line(&[(11.5, 9.0), (19.0, 9.0), (19.0, 16.5)]);
        }
        Mark::Heard => {
            line(&[(19.5, 8.5), (9.0, 19.0)]);
            line(&[(9.0, 11.5), (9.0, 19.0), (16.5, 19.0)]);
        }
        Mark::Failed => {
            line(&[(14.0, 7.0), (14.0, 16.0)]);
            painter.circle_filled(grid.at(14.0, 20.5), grid.len(1.25), color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_color_over_its_ground_is_solid() {
        let ground = Color32::WHITE;
        assert_eq!(
            solid(Color32::from_rgb(1, 2, 3), ground),
            Color32::from_rgb(1, 2, 3)
        );
        // Black at 50% over white is mid grey.
        let half = Color32::from_black_alpha(128);
        assert_eq!(solid(half, ground), Color32::from_rgb(127, 127, 127));
    }

    #[test]
    fn low_is_the_middle_wisp_medium_the_outer_pair_high_all_three() {
        assert_eq!(wisp_shares(0.0), [0.0, 0.0, 0.0]);
        assert_eq!(wisp_shares(1.0), [0.0, 1.0, 0.0]);
        assert_eq!(wisp_shares(2.0), [1.0, 0.0, 1.0]);
        assert_eq!(wisp_shares(3.0), [1.0, 1.0, 1.0]);
        // Easing from Low to Medium, the middle one fades as the outer pair comes in.
        assert_eq!(wisp_shares(1.5), [0.5, 0.5, 0.5]);
        assert_eq!(wisp_shares(2.5), [1.0, 0.5, 1.0]);
    }

    type Draw = fn(&Painter, Rect, &Ink, f32);

    /// Where a glyph's ink falls on its square, in grid units, drawn `side` points big:
    /// the bounds of every shape it paints, strokes included.
    fn ink_of(draw: Draw, ink: &Ink, level: f32, side: f32) -> Rect {
        let ctx = eframe::egui::Context::default();
        let square = Rect::from_min_size(Pos2::ZERO, vec2(side, side));
        let mut output = ctx.run_ui(eframe::egui::RawInput::default(), |ui| {
            draw(ui.painter(), square, ink, level);
        });
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        let painted = shapes
            .iter()
            .map(|clipped| clipped.shape.visual_bounding_rect())
            .fold(Rect::NOTHING, Rect::union);
        assert!(painted.is_positive(), "nothing painted");
        let units = UNIT / side;
        Rect::from_min_max(
            (painted.min.to_vec2() * units).to_pos2(),
            (painted.max.to_vec2() * units).to_pos2(),
        )
    }

    fn ink(track: Option<Color32>) -> Ink {
        Ink {
            body: Color32::GRAY,
            mist: Color32::GRAY,
            ground: Color32::WHITE,
            track,
        }
    }

    /// The models, and how far down the grid each body's ink reaches.
    const BODIES: [(&str, Draw, f32); 2] = [("Air 2", air2, 24.75), ("HVAC 2", hvac2, 24.25)];

    #[test]
    fn a_body_with_no_mist_and_no_tracks_sits_in_the_middle_of_its_square() {
        // A sidebar tile, the hero, and small enough for the stroke's floor.
        for side in [28.0, 72.0, 20.0] {
            for (model, draw, _) in BODIES {
                let painted = ink_of(draw, &ink(None), 0.0, side);
                let middle = painted.center().y;
                assert!(
                    (middle - UNIT / 2.0).abs() <= 0.5,
                    "{model} at {side} pt: ink {painted:?}, middle {middle}"
                );
            }
        }
    }

    #[test]
    fn with_mist_or_tracks_the_body_stays_in_the_lower_half() {
        for side in [28.0, 72.0] {
            for (model, draw, bottom) in BODIES {
                let mut drawn = vec![(ink(None), 1.0), (ink(None), 2.0), (ink(None), 3.0)];
                // The hero while it diffuses: tracks, even with no level.
                drawn.extend([0.0, 1.0, 3.0].map(|level| (ink(Some(Color32::LIGHT_GRAY)), level)));
                for (ink, level) in drawn {
                    let painted = ink_of(draw, &ink, level, side);
                    assert!(
                        (painted.bottom() - bottom).abs() < 0.01,
                        "{model} at {side} pt, level {level}, tracks {:?}: {painted:?}",
                        ink.track
                    );
                }
            }
        }
    }

    #[test]
    fn the_grid_scales_the_stroke_with_the_glyph() {
        let tile = Grid::of(Rect::from_min_size(Pos2::ZERO, vec2(28.0, 28.0)));
        assert_eq!(tile.stroke(), 1.5);
        let hero = Grid::of(Rect::from_min_size(Pos2::ZERO, vec2(56.0, 80.0)));
        assert_eq!(hero.stroke(), 3.0);
        // Centered in the rect it is given.
        assert_eq!(hero.at(14.0, 14.0), pos2(28.0, 40.0));
        // Never thinner than 1.25 pt.
        let small = Grid::of(Rect::from_min_size(Pos2::ZERO, vec2(14.0, 14.0)));
        assert_eq!(small.stroke(), 1.25);
    }
}
