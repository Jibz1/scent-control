//! The app icon, drawn in the style of macOS Big Sur and later: a rounded square with
//! continuous corners on Apple's icon grid (824 px of a 1024 px canvas, over a soft
//! shadow), in a gentle top-to-bottom gradient of the app's indigo, and on it the Air 2
//! of the sidebar's glyph in white, its three wisps of mist rising. The glyph's geometry
//! is the one in [`glyphs`], so that the icon and the app match: its capsule is filled
//! out to the outer edge of the glyph's stroke, and its outlet is a slot cut into it.
//! At 16 and 32 px, where that would blur, the glyph is simplified and set on whole
//! pixels.
//!
//! The bundle's `AppIcon.icns` is made from it. The ignored `render_app_icon` test
//! draws it with the snapshots' headless renderer at each size an `.iconset` needs,
//! writes them to `target/AppIcon.iconset`, and runs `iconutil` to write
//! `apps/desktop/macos/AppIcon.icns`, which `scripts/bundle-macos.sh` copies into the
//! bundle. After changing the icon, run
//! `cargo test -p scent-desktop render_app_icon -- --ignored` and commit the new file.

use eframe::egui::{
    self, Color32, CornerRadius, Mesh, Painter, Pos2, Rect, Shape, Stroke, Vec2, vec2,
};
use eframe::epaint::{PathShape, RectShape, TessellationOptions, Tessellator};

use super::glyphs::{self, AIR2_BODY, AIR2_OUTLET, AIR2_SIDES, STROKE, UNIT};
use super::theme::Palette;

/// Apple's icon grid for the Mac, in pixels of its 1024 px canvas: a tile 824 px square
/// in the middle, with continuous corners of radius 185.4 px.
const CANVAS: f32 = 1024.0;
const TILE: f32 = 824.0;
const TILE_RADIUS: f32 = 185.4;

/// The tile's shadow, in pixels of the canvas: black at 30%, 10 px down, softened over
/// 20 px.
const SHADOW_DROP: f32 = 10.0;
const SHADOW_BLUR: f32 = 20.0;
const SHADOW_ALPHA: u8 = 77;

/// How big the glyph's 28-unit grid is on the canvas, centered on the tile.
const GLYPH: f32 = 640.0;
/// The wisps' stroke, in grid units, at the foot and at the tip: a little heavier than
/// the sidebar's at the foot, beside a body that is filled rather than drawn in outline,
/// and thinning as the mist rises.
const WISP_STROKE: [f32; 2] = [2.0, 1.25];

/// A continuous corner of radius 1, as UIKit draws its rounded rectangles and Apple
/// shapes its icons: where a circle's arc would meet the edges abruptly, the curve eases
/// into them, reaching 1.53 radii along each from the corner. It is two cubic Béziers,
/// the second symmetric about the corner's diagonal, and then the first mirrored back to
/// the other edge. (UIKit's path has a short line between the first two; it is folded
/// into the first here, so that the outline stays convex.) Each point is how far it is
/// from the corner back along the edge the corner leaves (x), and in from that edge (y).
const EASE: [Vec2; 4] = [
    vec2(1.528_665, 0.0),
    vec2(1.088_493, 0.0),
    vec2(0.868_407, 0.0),
    ARC[0],
];
const ARC: [Vec2; 4] = [
    vec2(0.631_494, 0.074_911),
    vec2(0.372_824, 0.169_059),
    vec2(0.169_059, 0.372_824),
    vec2(0.074_911, 0.631_494),
];

/// A color that runs down `span` (its top and bottom) from the first of `colors` to
/// the second, blended in gamma space as egui blends.
#[derive(Debug, Clone, Copy)]
struct Gradient {
    span: [f32; 2],
    colors: [Color32; 2],
}

impl Gradient {
    fn at(&self, y: f32) -> Color32 {
        let [top, bottom] = self.span;
        let [from, to] = self.colors;
        from.lerp_to_gamma(to, ((y - top) / (bottom - top)).clamp(0.0, 1.0))
    }
}

/// The Air 2 as it is drawn at 16 and 32 px, where the glyph's own proportions blur:
/// its features placed by hand on whole pixels, in pixels of the canvas. The capsule is
/// squatter, leaving the mist room to show; at 16 px the wisps are straight and the
/// outlet goes.
struct Small {
    /// The canvas, in pixels.
    size: u32,
    /// Where the tile runs, across and down. At 16 px it is 13 px, half a pixel up and to
    /// the left of the canvas's middle, so that its own middle is a pixel's, and the
    /// middle wisp there one pixel wide.
    tile: [f32; 2],
    /// The capsule, and its outlet if it shows: left, top, right and bottom.
    body: [f32; 4],
    outlet: Option<[f32; 4]>,
    /// Each wisp's line: where it runs across, and its foot and tip, down the canvas.
    /// Its ends are round, so it reaches half its width past them.
    wisps: [[f32; 3]; 3],
    wisp_width: f32,
    /// How far each wisp sways either side of its line, as the glyph's do.
    sway: f32,
}

const SMALL: [Small; 2] = [
    Small {
        size: 16,
        tile: [1.0, 14.0],
        body: [6.0, 8.0, 9.0, 12.0],
        outlet: None,
        wisps: [[4.5, 5.5, 3.5], [7.5, 6.5, 4.5], [10.5, 5.5, 3.5]],
        wisp_width: 1.0,
        sway: 0.0,
    },
    Small {
        size: 32,
        tile: [3.0, 29.0],
        body: [13.0, 18.0, 19.0, 26.0],
        outlet: Some([14.5, 20.0, 17.5, 21.0]),
        wisps: [[12.0, 14.0, 7.0], [16.0, 15.0, 8.0], [20.0, 14.0, 7.0]],
        wisp_width: 1.5,
        sway: 0.6,
    },
];

/// Paints the icon to fill `canvas`, a square that is the whole of Apple's canvas,
/// margins and shadow included: as [`SMALL`] has it at 16 and 32 px, and otherwise with
/// the glyph's geometry.
pub fn paint(painter: &Painter, canvas: Rect) {
    let scale = canvas.width() / CANVAS;
    let pixel = 1.0 / painter.pixels_per_point();
    let small = SMALL
        .iter()
        .find(|small| (canvas.width() / pixel).round() as u32 == small.size);
    let at = |x: f32, y: f32| canvas.min + vec2(x, y) * pixel;
    let tile = match small {
        Some(Small {
            tile: [near, far], ..
        }) => Rect::from_min_max(at(*near, *near), at(*far, *far)),
        None => Rect::from_center_size(canvas.center(), Vec2::splat(TILE * scale)),
    };
    let radius = tile.width() * TILE_RADIUS / TILE;
    let shadow = RectShape::filled(
        tile.translate(vec2(0.0, SHADOW_DROP * scale)),
        CornerRadius::from(radius),
        Color32::from_black_alpha(SHADOW_ALPHA),
    );
    painter.add(shadow.with_blur_width(SHADOW_BLUR * scale));
    // A touch lighter than the dark appearance's accent at the top, and a touch darker
    // than the light one's at the bottom.
    let ground = Gradient {
        span: [tile.top(), tile.bottom()],
        colors: [
            Palette::DARK.accent.lerp_to_gamma(Color32::WHITE, 0.08),
            Palette::LIGHT.accent.lerp_to_gamma(Color32::BLACK, 0.12),
        ],
    };
    fill(painter, squircle(tile, radius), ground);
    match small {
        Some(small) => {
            let rect = |[left, top, right, bottom]: [f32; 4]| {
                Rect::from_min_max(at(left, top), at(right, bottom))
            };
            body(painter, rect(small.body), small.outlet.map(rect), ground);
            let width = small.wisp_width * pixel;
            for (i, [x, foot, tip]) in small.wisps.into_iter().enumerate() {
                if small.sway == 0.0 {
                    // Straight, as a capsule: its sides are then crisp, where egui's
                    // smallest round ends would blur into the pixels beside them.
                    let rect = Rect::from_min_max(
                        at(x, tip) - Vec2::splat(width / 2.0),
                        at(x, foot) + Vec2::splat(width / 2.0),
                    );
                    fill(
                        painter,
                        capsule(rect),
                        mist(at(x, foot), at(x, tip), ground),
                    );
                    continue;
                }
                // The left one sways the other way from the right one, as the glyph's do.
                let side = if i == 0 { -1.0 } else { 1.0 };
                let steps = 12;
                let line = (0..=steps)
                    .map(|step| {
                        let along = step as f32 / steps as f32;
                        let sway = side * small.sway * (std::f32::consts::TAU * along).sin();
                        at(x + sway, foot - (foot - tip) * along)
                    })
                    .collect();
                wisp(painter, line, [width, width], ground);
            }
        }
        None => diffuser(painter, tile.center(), GLYPH * scale, ground),
    }
}

/// The Air 2 on its `ground`, as the glyph draws it, with its grid `side` square and
/// centered on `center`.
fn diffuser(painter: &Painter, center: Pos2, side: f32, ground: Gradient) {
    let unit = side / UNIT;
    let origin = center - Vec2::splat(side / 2.0);
    let at = |x: f32, y: f32| origin + vec2(x, y) * unit;
    // The capsule, out to the outer edge of the glyph's stroke, over its shadow on the
    // tile, and the outlet as wide as the glyph's slit with its round ends.
    let edge = STROKE / 2.0;
    let [top, bottom] = AIR2_BODY;
    let [left, right] = AIR2_SIDES;
    let capsule = Rect::from_min_max(at(left - edge, top - edge), at(right + edge, bottom + edge));
    let shadow = RectShape::filled(
        capsule.translate(vec2(0.0, 0.5 * unit)),
        CornerRadius::from(capsule.width() / 2.0),
        Color32::from_black_alpha(48),
    );
    painter.add(shadow.with_blur_width(1.5 * unit));
    let [[from, y], [to, _]] = AIR2_OUTLET;
    let outlet = Rect::from_min_max(at(from - edge, y - edge), at(to + edge, y + edge));
    body(painter, capsule, Some(outlet), ground);
    for i in 0..3 {
        let line = glyphs::wisp_line(i, 1.0)
            .into_iter()
            .map(|point| at(point.x, point.y))
            .collect();
        wisp(painter, line, WISP_STROKE.map(|width| width * unit), ground);
    }
}

/// The capsule filling `rect`: white, turning the faintest indigo toward its foot, with
/// its `outlet` a slot in it, deeper than the `ground` around it.
fn body(painter: &Painter, rect: Rect, outlet: Option<Rect>, ground: Gradient) {
    let white = Gradient {
        span: [rect.top(), rect.bottom()],
        colors: [
            Color32::WHITE,
            Color32::WHITE.lerp_to_gamma(Palette::LIGHT.accent, 0.1),
        ],
    };
    fill(painter, capsule(rect), white);
    if let Some(slot) = outlet {
        let hollow = ground
            .at(slot.center().y)
            .lerp_to_gamma(Color32::BLACK, 0.2);
        painter.rect_filled(slot, CornerRadius::from(slot.height() / 2.0), hollow);
    }
}

/// A wisp along `line`, from its foot up, with round ends: white, thinning into the
/// `ground` as it rises, and from the first of `widths` at its foot to the second at its
/// tip. egui's strokes are one width, so the wisp is a strip of its own: across each
/// point of the line, its edges softened over a pixel as egui softens its own.
fn wisp(painter: &Painter, line: Vec<Pos2>, widths: [f32; 2], ground: Gradient) {
    let (foot, tip) = (line[0], line[line.len() - 1]);
    let mist = mist(foot, tip, ground);
    let feather = 1.0 / painter.pixels_per_point();
    let last = line.len() - 1;
    let mut mesh = Mesh::default();
    for (i, &point) in line.iter().enumerate() {
        let across = (line[(i + 1).min(last)] - line[i.saturating_sub(1)])
            .normalized()
            .rot90();
        let half = egui::lerp(widths[0]..=widths[1], i as f32 / last as f32) / 2.0;
        let (core, rim) = ((half - feather / 2.0).max(0.0), half + feather / 2.0);
        let color = mist.at(point.y);
        for (offset, color) in [
            (rim, Color32::TRANSPARENT),
            (core, color),
            (-core, color),
            (-rim, Color32::TRANSPARENT),
        ] {
            mesh.colored_vertex(point + across * offset, color);
        }
        if i > 0 {
            let [before, here] = [i - 1, i].map(|i| 4 * i as u32);
            for k in 0..3 {
                mesh.add_triangle(before + k, before + k + 1, here + k);
                mesh.add_triangle(before + k + 1, here + k + 1, here + k);
            }
        }
    }
    painter.add(Shape::mesh(mesh));
    // The round ends, as discs filled as the tile is: egui's smallest circles are drawn
    // from blurred images of discs, which would bleed into the pixels around them.
    for (end, width) in [(foot, widths[0]), (tip, widths[1])] {
        let disc = Rect::from_center_size(end, Vec2::splat(width));
        fill(painter, capsule(disc), mist);
    }
}

/// A wisp's color, from its `foot` to its `tip`: white, and then less so, a little of the
/// `ground` showing through as the mist thins. The ground runs on down the tile, but
/// over the short height of a wisp the two blend as a straight run between their ends.
fn mist(foot: Pos2, tip: Pos2, ground: Gradient) -> Gradient {
    Gradient {
        span: [foot.y, tip.y],
        colors: [
            Color32::WHITE,
            Color32::WHITE.lerp_to_gamma(ground.at(tip.y), 0.3),
        ],
    }
}

/// Fills the convex `outline` with `color` at each height, its edge softened over a
/// pixel as egui softens its own fills. egui has no gradients, so this tessellates the
/// fill as egui would, in white, and then colors each vertex for its height; across
/// each triangle the color then runs as the gradient does.
fn fill(painter: &Painter, outline: Vec<Pos2>, color: Gradient) {
    let options = TessellationOptions::default();
    let mut tessellator = Tessellator::new(painter.pixels_per_point(), options, [1, 1], vec![]);
    let mut mesh = Mesh::default();
    let shape = PathShape::convex_polygon(outline, Color32::WHITE, Stroke::NONE);
    tessellator.tessellate_path(&shape, &mut mesh);
    for vertex in &mut mesh.vertices {
        let coverage = f32::from(vertex.color.a()) / 255.0;
        vertex.color = color.at(vertex.pos.y).gamma_multiply(coverage);
    }
    painter.add(Shape::mesh(mesh));
}

/// A point on the cubic Bézier through `points`, `t` of the way along it.
fn cubic([p0, p1, p2, p3]: [Vec2; 4], t: f32) -> Vec2 {
    let u = 1.0 - t;
    p0 * (u * u * u) + p1 * (3.0 * u * u * t) + p2 * (3.0 * u * t * t) + p3 * (t * t * t)
}

/// A rounded square filling `rect`, with continuous corners of `radius`: its outline,
/// clockwise from the top edge.
fn squircle(rect: Rect, radius: f32) -> Vec<Pos2> {
    let steps = 16;
    let curve = |bezier| (0..=steps).map(move |step| cubic(bezier, step as f32 / steps as f32));
    let ease: Vec<Vec2> = curve(EASE).collect();
    // The curves meet end to start, so each meeting point is taken once.
    let corner: Vec<Vec2> = ease
        .iter()
        .copied()
        .chain(curve(ARC).skip(1))
        .chain(
            ease.iter()
                .rev()
                .skip(1)
                .map(|point| vec2(point.y, point.x)),
        )
        .collect();
    // Each corner in turn: where it is, the way back along the edge it leaves, and the
    // way in from that edge.
    let corners = [
        (rect.right_top(), vec2(-1.0, 0.0), vec2(0.0, 1.0)),
        (rect.right_bottom(), vec2(0.0, -1.0), vec2(-1.0, 0.0)),
        (rect.left_bottom(), vec2(1.0, 0.0), vec2(0.0, -1.0)),
        (rect.left_top(), vec2(0.0, 1.0), vec2(1.0, 0.0)),
    ];
    corners
        .into_iter()
        .flat_map(|(at, back, inward)| {
            corner
                .iter()
                .map(move |point| at + (back * point.x + inward * point.y) * radius)
        })
        .collect()
}

/// An upright capsule filling `rect`, or a disc in a square: its outline, clockwise from
/// its left side, over the top.
fn capsule(rect: Rect) -> Vec<Pos2> {
    let radius = rect.width() / 2.0;
    let ends = [
        (rect.center_top() + vec2(0.0, radius), std::f32::consts::PI),
        (rect.center_bottom() - vec2(0.0, radius), 0.0),
    ];
    let steps = 32;
    let mut outline: Vec<Pos2> = ends
        .into_iter()
        .flat_map(|(center, start)| {
            (0..=steps).map(move |step| {
                let angle = start + std::f32::consts::PI * step as f32 / steps as f32;
                center + radius * Vec2::angled(angle)
            })
        })
        .collect();
    // In a disc the two halves meet end to end. Each point where they meet is kept
    // once: egui's fill would make a spike of the edge between two that all but meet.
    let apart = |a: Pos2, b: Pos2| a.distance(b) > radius * 1e-3;
    outline.dedup_by(|point, before| !apart(*point, *before));
    if outline.len() > 1 && !apart(outline[0], outline[outline.len() - 1]) {
        outline.pop();
    }
    outline
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use eframe::egui::{LayerId, pos2};
    use egui_kittest::Harness;

    use super::*;

    fn target() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target")
    }

    /// The bounds of what `draw` paints on a canvas `side` points square.
    fn painted(side: f32, draw: impl Fn(&Painter, Rect)) -> Rect {
        let ctx = egui::Context::default();
        let canvas = Rect::from_min_size(Pos2::ZERO, Vec2::splat(side));
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| draw(ui.painter(), canvas));
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        shapes
            .iter()
            .map(|clipped| clipped.shape.visual_bounding_rect())
            .fold(Rect::NOTHING, Rect::union)
    }

    #[test]
    fn the_tile_is_on_apples_grid() {
        let tile = Rect::from_min_max(pos2(100.0, 100.0), pos2(924.0, 924.0));
        let outline = squircle(tile, TILE_RADIUS);
        let bounds = Rect::from_points(&outline);
        assert!(
            (bounds.min - tile.min).length() < 1e-3 && (bounds.max - tile.max).length() < 1e-3,
            "{bounds:?}"
        );
        // Convex, all the way round: every turn is the same way.
        let n = outline.len();
        for i in 0..n {
            let [a, b, c] = [0, 1, 2].map(|k| outline[(i + k) % n]);
            let turn = (b - a).x * (c - b).y - (b - a).y * (c - b).x;
            assert!(turn >= -1e-3, "turns back at {b:?}");
        }
        // On the diagonal a corner is where a circle of its radius would be, within a
        // pixel, but it eases into the edges, leaving them 1.53 radii from the corner
        // rather than 1.
        let diagonal = outline
            .iter()
            .map(|point| point.x - tile.left() + (point.y - tile.top()))
            .fold(f32::INFINITY, f32::min);
        let circle = 2.0 * TILE_RADIUS * (1.0 - std::f32::consts::FRAC_1_SQRT_2);
        assert!(
            (diagonal - circle).abs() < 1.0,
            "{diagonal} against {circle}"
        );
        let leaves = outline
            .iter()
            .filter(|point| point.y - tile.top() < 1e-3)
            .map(|point| point.x - tile.left())
            .fold(f32::INFINITY, f32::min);
        assert!((leaves - 1.528_665 * TILE_RADIUS).abs() < 1e-2, "{leaves}");
    }

    #[test]
    fn the_diffuser_is_centered_on_the_tile_and_inside_it() {
        let side = CANVAS;
        let ground = Gradient {
            span: [0.0, side],
            colors: [Color32::BLUE, Color32::BLUE],
        };
        let center = pos2(side / 2.0, side / 2.0);
        let ink = painted(side, |painter, _| {
            diffuser(painter, center, GLYPH, ground);
        });
        let unit = GLYPH / UNIT;
        assert!((ink.center().x - center.x).abs() < 0.25 * unit, "{ink:?}");
        assert!((ink.center().y - center.y).abs() < 0.75 * unit, "{ink:?}");
        // Well inside the tile, clear of its corners.
        let tile = Rect::from_center_size(center, Vec2::splat(TILE));
        assert!(tile.shrink(TILE_RADIUS * 0.5).contains_rect(ink), "{ink:?}");
    }

    #[test]
    fn a_disc_is_filled_without_spikes() {
        // Round ends of many widths, at many places between pixels, as the icon puts
        // them at each size and in the strip of sizes.
        let ground = Gradient {
            span: [0.0, 1.0],
            colors: [Color32::WHITE, Color32::WHITE],
        };
        for i in 0..200 {
            let step = i as f32;
            let center = pos2(37.0 + 0.137 * step, 51.0 + 0.291 * step);
            let disc = Rect::from_center_size(center, Vec2::splat(0.5 + 0.25 * step));
            let ink = painted(200.0, |painter, _| {
                fill(painter, capsule(disc), ground);
            });
            assert!(disc.expand(1.0).contains_rect(ink), "{disc:?}: {ink:?}");
        }
    }

    #[test]
    fn the_small_sizes_are_centered_on_their_tiles() {
        for small in &SMALL {
            let [near, far] = small.tile;
            let middle = (near + far) / 2.0;
            let size = small.size as f32;
            assert!(near > 0.0 && far < size, "{size} px: tile {:?}", small.tile);
            let [left, top, right, bottom] = small.body;
            assert_eq!((left + right) / 2.0, middle, "{size} px: body");
            if let Some([left, _, right, _]) = small.outlet {
                assert_eq!((left + right) / 2.0, middle, "{size} px: outlet");
            }
            let [
                [outer, foot, tip],
                [inner, ..],
                [other, other_foot, other_tip],
            ] = small.wisps;
            assert_eq!(inner, middle, "{size} px: middle wisp");
            assert_eq!(outer + other, 2.0 * middle, "{size} px: outer wisps");
            assert_eq!(
                [foot, tip],
                [other_foot, other_tip],
                "{size} px: outer wisps"
            );
            // As much tile above the glyph as below it, to within half a pixel.
            let ink_top = small
                .wisps
                .map(|[_, _, tip]| tip)
                .into_iter()
                .fold(far, f32::min)
                - small.wisp_width / 2.0;
            assert!(top > ink_top, "{size} px: the mist runs into the body");
            let (above, below) = (ink_top - near, far - bottom);
            assert!(
                (above - below).abs() <= 0.5,
                "{size} px: {above} above, {below} below"
            );
        }
    }

    /// Renders `draw` on a clear canvas of `size` pixels with the snapshots' renderer,
    /// and writes it to `path` as a PNG. The renderer hands back colors premultiplied by
    /// their alpha; a PNG's are not, so they are divided back out.
    fn render(path: &Path, size: Vec2, mut draw: impl FnMut(&Painter, Rect)) {
        let mut frames = 0;
        let mut harness = Harness::builder()
            .with_size(size)
            .with_pixels_per_point(1.0)
            .wgpu()
            .build_ui(move |ui| {
                frames += 1;
                if frames == 1 {
                    // The harness's panel is clear, so that only the icon is drawn.
                    ui.ctx()
                        .all_styles_mut(|style| style.visuals.panel_fill = Color32::TRANSPARENT);
                    ui.ctx().request_repaint();
                    return;
                }
                let canvas = ui.ctx().content_rect();
                draw(&ui.ctx().layer_painter(LayerId::background()), canvas);
            });
        harness.run_steps(2);
        let mut image = harness.render().expect("render the icon");
        for pixel in image.pixels_mut() {
            let [r, g, b, a] = pixel.0;
            if a > 0 && a < 255 {
                let a = u32::from(a);
                let straight = |c: u8| ((u32::from(c) * 255 + a / 2) / a).min(255) as u8;
                pixel.0 = [straight(r), straight(g), straight(b), a as u8];
            }
        }
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image.save(path).unwrap();
        println!("wrote {}", path.display());
    }

    #[test]
    #[ignore = "writes the icon's PNGs and apps/desktop/macos/AppIcon.icns; run on demand"]
    fn render_app_icon() {
        // Each size of an iconset, at 1x and 2x, named as iconutil wants them.
        let iconset = target().join("AppIcon.iconset");
        for points in [16, 32, 128, 256, 512] {
            for (scale, suffix) in [(1, ""), (2, "@2x")] {
                let name = format!("icon_{points}x{points}{suffix}.png");
                let pixels = (points * scale) as f32;
                render(&iconset.join(name), Vec2::splat(pixels), paint);
            }
        }
        let icns = Path::new(env!("CARGO_MANIFEST_DIR")).join("macos/AppIcon.icns");
        let built = Command::new("iconutil")
            .arg("-c")
            .arg("icns")
            .arg(&iconset)
            .arg("-o")
            .arg(&icns)
            .status()
            .expect("run iconutil");
        assert!(built.success(), "iconutil: {built}");
        println!("wrote {}", icns.display());

        // The small sizes as they are seen, on the window colors of the light and dark
        // appearances: 16 px in a list, 32 in Finder's icons, 64 and 128 in the Dock.
        let sizes = [16.0, 32.0, 64.0, 128.0];
        let gap = 24.0;
        let row = 128.0 + 2.0 * gap;
        let width = gap + sizes.iter().map(|size| size + gap).sum::<f32>();
        let strip = target().join("snapshots/app_icon_sizes.png");
        render(&strip, vec2(width, 2.0 * row), |painter, canvas| {
            for (i, palette) in [Palette::LIGHT, Palette::DARK].into_iter().enumerate() {
                let band = Rect::from_min_size(
                    canvas.min + vec2(0.0, row * i as f32),
                    vec2(canvas.width(), row),
                );
                painter.rect_filled(band, 0, palette.window);
                let mut left = band.left() + gap;
                for size in sizes {
                    let top = (band.center().y - size / 2.0).round();
                    paint(
                        painter,
                        Rect::from_min_size(pos2(left, top), Vec2::splat(size)),
                    );
                    left += size + gap;
                }
            }
        });
    }
}
