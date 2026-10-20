//! The app's drawn glyphs, since egui can't use SF Symbols: the Air 2 (a capsule) and
//! the HVAC 2 (a box with vent slits), each with up to three mist arcs on its trailing
//! side for its level, as SF Symbols' speaker draws its volume;
//! signal bars; the log's sent and heard chevrons; the large diffuser of an empty state;
//! and the rounded tile they sit on in the sidebar.
//!
//! Each is drawn on a grid of 28 units, the size of a sidebar tile, scaled to the square
//! it is given: strokes are 1.5 units (1.5 pt on a tile, scaled with the glyph, never
//! thinner than 1.25 pt), with round ends and joins, and corners rounded as the tiles'
//! are. They are painted, not laid out: they have no accessibility node of their own,
//! since the words beside them say what they show.

use std::f32::consts::PI;

use eframe::egui::{
    Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, lerp, pos2, vec2,
};
use eframe::epaint::PathShape;

/// The grid the glyphs are drawn on, and the stroke on it.
const UNIT: f32 = 28.0;
const STROKE: f32 = 1.5;
/// The thinnest a stroke gets, at small sizes, so that it stays crisp.
const MIN_STROKE: f32 = 1.25;

/// A tile's corner radius, as a share of its side: 6 pt on a 28 pt tile, the radius of
/// the app's controls.
const TILE_CORNER: f32 = 6.0 / UNIT;

/// The mist arcs: how far the first one is from the body's edge, the space between
/// them, and how far each sweeps either side of level, in radians.
const ARC_CLEARANCE: f32 = 3.0;
const ARC_SPACING: f32 = 2.75;
const ARC_SWEEP: f32 = 40.0 * PI / 180.0;

/// Where a body's middle is: left of the grid's middle, so that the body and its three
/// arcs together are centered.
const BODY_X: f32 = 9.5;

/// The colors a diffuser glyph is drawn in.
#[derive(Debug, Clone, Copy)]
pub struct Ink {
    /// The body.
    pub body: Color32,
    /// The mist arcs.
    pub arcs: Color32,
    /// What the glyph is drawn on. The glyph's colors are made solid over it, so that
    /// where a stroke's round end overlaps it nothing shows darker.
    pub ground: Color32,
    /// The arcs past the level, as faint tracks, as signal bars show the bars not
    /// filled; `None` leaves them out. The hero shows them, so that its scale of three
    /// is always there and its body never stands alone.
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

/// Points along the arc of `radius` around `center`, from `from` to `to` radians
/// (0 points right, angles grow clockwise on screen).
fn arc(center: Pos2, radius: f32, from: f32, to: f32) -> Vec<Pos2> {
    let steps = 16;
    (0..=steps)
        .map(|step| {
            let angle = lerp(from..=to, step as f32 / steps as f32);
            center + vec2(angle.cos(), angle.sin()) * radius
        })
        .collect()
}

/// Up to three mist arcs on the trailing side of `center`, the first `first` units out.
/// `level` may be between whole numbers while the arcs ease: an arc part-way in is drawn
/// part of the way from the ground's color to its own, and a little shorter.
fn mist(painter: &Painter, grid: &Grid, ink: &Ink, level: f32, center: (f32, f32), first: f32) {
    let center = grid.at(center.0, center.1);
    let color = solid(ink.arcs, ink.ground);
    for i in 0..3 {
        let shown = (level - i as f32).clamp(0.0, 1.0);
        // From its track, or from nothing (the ground), and then a little shorter.
        let (from, sweep) = match ink.track {
            Some(track) => (solid(track, ink.ground), ARC_SWEEP),
            None if shown <= 0.0 => continue,
            None => (ink.ground, ARC_SWEEP * (0.7 + 0.3 * shown)),
        };
        let ink = from.lerp_to_gamma(color, shown);
        let radius = grid.len(first + ARC_SPACING * i as f32);
        let points = arc(center, radius, -sweep, sweep);
        round_line(painter, points, grid.stroke(), ink);
    }
}

/// The Air 2: an upright capsule with its mist outlet near the top, and `level` mist
/// arcs (0 to 3) after it.
pub fn air2(painter: &Painter, rect: Rect, ink: &Ink, level: f32) {
    let grid = Grid::of(rect);
    let body = solid(ink.body, ink.ground);
    // 6 by 13 units: a capsule, so its radius is half its width.
    let capsule = grid.rect(BODY_X - 3.0, 7.5, BODY_X + 3.0, 20.5);
    painter.rect_stroke(
        capsule,
        CornerRadius::from(grid.len(3.0)),
        Stroke::new(grid.stroke(), body),
        StrokeKind::Middle,
    );
    painter.circle_filled(grid.at(BODY_X, 10.75), grid.len(0.9), body);
    let first = 3.0 + ARC_CLEARANCE;
    mist(painter, &grid, ink, level, (BODY_X, 14.0), first);
}

/// The HVAC 2: a low box with two vent slits, and `level` mist arcs (0 to 3) after it.
pub fn hvac2(painter: &Painter, rect: Rect, ink: &Ink, level: f32) {
    let grid = Grid::of(rect);
    let body = solid(ink.body, ink.ground);
    // 9 by 10 units, with the tiles' corner language.
    let case = grid.rect(BODY_X - 4.5, 9.0, BODY_X + 4.5, 19.0);
    painter.rect_stroke(
        case,
        CornerRadius::from(grid.len(2.0)),
        Stroke::new(grid.stroke(), body),
        StrokeKind::Middle,
    );
    for y in [12.25, 15.75] {
        let slit = vec![grid.at(BODY_X - 2.0, y), grid.at(BODY_X + 2.0, y)];
        round_line(painter, slit, grid.stroke(), body);
    }
    let first = 4.5 + ARC_CLEARANCE;
    mist(painter, &grid, ink, level, (BODY_X, 14.0), first);
}

/// The large diffuser of an empty state: the Air 2 with its three arcs faint, waiting.
pub fn empty_diffuser(painter: &Painter, rect: Rect, color: Color32, ground: Color32) {
    let faint = solid(color, ground).lerp_to_gamma(ground, 0.6);
    let ink = Ink {
        body: color,
        arcs: faint,
        ground,
        track: None,
    };
    air2(painter, rect, &ink, 3.0);
}

/// Where to draw a diffuser glyph that stays still (a tile, an empty state) so that it
/// looks centered in `rect`: with no arcs, its body alone is moved to the middle. The
/// hero doesn't use it, so that its body stays put as the arcs ease in and out.
pub fn still(rect: Rect, level: f32) -> Rect {
    if level > 0.0 {
        return rect;
    }
    let scale = rect.width().min(rect.height()) / UNIT;
    rect.translate(vec2((UNIT / 2.0 - BODY_X) * scale, 0.0))
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

/// Which way a log row's chevron points: what this app sent goes out to the diffuser,
/// what it heard comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pointing {
    /// Sent: ›
    Out,
    /// Heard: ‹
    In,
}

/// A chevron in `rect`, centered by eye: its point carries more weight than its open
/// side, so it sits a little toward the open side.
pub fn chevron(painter: &Painter, rect: Rect, color: Color32, pointing: Pointing) {
    let grid = Grid::of(rect);
    // 7 units wide and 14 tall, the point 1 unit off center toward the open side.
    let (tip, open) = match pointing {
        Pointing::Out => (17.5, 10.5),
        Pointing::In => (10.5, 17.5),
    };
    let shift = if pointing == Pointing::Out { -0.5 } else { 0.5 };
    let points = vec![
        grid.at(open + shift, 7.0),
        grid.at(tip + shift, 14.0),
        grid.at(open + shift, 21.0),
    ];
    round_line(painter, points, grid.stroke(), color);
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
