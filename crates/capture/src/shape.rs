//! Shapes dragged with a tool instead of drawn by hand: a box or an arrow from where the pen went
//! down to where it came up.
//!
//! While the pen is down a shape's stroke holds two points, the drag's start and where it is now
//! ([`drag`]). When it lifts, [`finished`] replaces them with the shape's outline, sampled every
//! couple of points, so everything that draws ink (the overlay, keyframes, the contact sheet, an
//! older Scrawl reading the bundle) draws the shape without knowing about tools. The stroke's
//! [`Tool`] says what it is, for marks ("drew an arrow pointing to …").

use std::f32::consts::PI;
use std::sync::{Mutex, PoisonError};

use crate::model::{SCALE, Stroke, Tool};

/// A box's corner radius, and how far its outline runs on past where it began, in the target's
/// points.
const BOX_RADIUS: f32 = 8.0;
const BOX_OVERLAP: f32 = 16.0;
/// An arrow's barbs: their length, at most this share of the shaft, and their angle off it.
const HEAD_LEN: f32 = 24.0;
const HEAD_SHARE: f32 = 0.4;
const HEAD_ANGLE: f32 = 30.0;
/// How far apart the outline's points are at most, in points, and the most it has.
const STEP: f32 = 2.0;
const MAX_POINTS: f32 = 1_200.0;

/// The tool ink that starts now draws with.
pub fn current(tool: &Mutex<Tool>) -> Tool {
    *tool.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Moves a shape's drag to `(x, y)` (in `0..=SCALE`) at `t_ms`: its first point stays where the
/// pen went down, its second follows the pointer.
pub fn drag(stroke: &mut Stroke, [x, y]: [i32; 2], t_ms: u64) {
    stroke.pts.truncate(3);
    let dt = t_ms.saturating_sub(stroke.t0) as i32;
    stroke.pts.extend_from_slice(&[x, y, dt]);
    stroke.t1 = t_ms;
}

/// The stroke as it's kept: a shape's drag becomes its outline, every point at the drag's last
/// moment so it shows whole. Ink, and a shape already finished, come back unchanged.
pub fn finished(mut stroke: Stroke) -> Stroke {
    let &[x0, y0, _, x1, y1, dt] = stroke.pts.as_slice() else {
        return stroke;
    };
    let (w, h) = (stroke.fw.max(1) as f32, stroke.fh.max(1) as f32);
    let to_points = |x: i32, y: i32| [x as f32 / SCALE as f32 * w, y as f32 / SCALE as f32 * h];
    let outline = outline(stroke.tool, to_points(x0, y0), to_points(x1, y1));
    if outline.len() < 3 {
        return stroke;
    }
    stroke.pts = outline
        .iter()
        .flat_map(|[x, y]| {
            [
                (x / w * SCALE as f32).round() as i32,
                (y / h * SCALE as f32).round() as i32,
                dt,
            ]
        })
        .collect();
    stroke
}

/// What `tool` draws dragged from `from` to `to` (points), as points along its outline: a box's
/// rounded rectangle round from its top edge, or an arrow's shaft then its two barbs.
/// Empty for the pen.
pub fn outline(tool: Tool, from: [f32; 2], to: [f32; 2]) -> Vec<[f32; 2]> {
    let corners: Vec<[f32; 2]> = match tool {
        Tool::Pen => return Vec::new(),
        Tool::Box => rounded_box(from, to),
        Tool::Arrow => {
            let len = distance(from, to);
            if len < 1.0 {
                return Vec::new();
            }
            let back = [(from[0] - to[0]) / len, (from[1] - to[1]) / len];
            let head = HEAD_LEN.min(HEAD_SHARE * len);
            let barb = |degrees: f32| {
                let (sin, cos) = degrees.to_radians().sin_cos();
                [
                    to[0] + head * (back[0] * cos - back[1] * sin),
                    to[1] + head * (back[0] * sin + back[1] * cos),
                ]
            };
            vec![from, to, barb(HEAD_ANGLE), to, barb(-HEAD_ANGLE)]
        }
    };
    let length: f32 = corners.windows(2).map(|p| distance(p[0], p[1])).sum();
    let step = STEP.max(length / MAX_POINTS);
    let mut points = vec![corners[0]];
    for pair in corners.windows(2) {
        let n = (distance(pair[0], pair[1]) / step).ceil().max(1.0) as usize;
        points.extend((1..=n).map(|i| lerp(pair[0], pair[1], i as f32 / n as f32)));
    }
    points
}

/// A box's corners, each rounded by a quarter circle, from the middle of its top edge round and
/// a little past it again: ink renderers thin a stroke's first points, which the overlap covers.
fn rounded_box(from: [f32; 2], to: [f32; 2]) -> Vec<[f32; 2]> {
    let (x0, x1) = (from[0].min(to[0]), from[0].max(to[0]));
    let (y0, y1) = (from[1].min(to[1]), from[1].max(to[1]));
    let r = BOX_RADIUS.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0);
    let middle = (x0 + x1) / 2.0;
    // Each corner's centre and the angle its arc starts at, clockwise on screen.
    let arcs = [
        ([x1 - r, y0 + r], -0.5 * PI),
        ([x1 - r, y1 - r], 0.0),
        ([x0 + r, y1 - r], 0.5 * PI),
        ([x0 + r, y0 + r], PI),
    ];
    let mut corners = vec![[middle, y0]];
    for (centre, start) in arcs {
        corners.extend((0..=6).map(|i| {
            let angle = start + 0.5 * PI * i as f32 / 6.0;
            [centre[0] + r * angle.cos(), centre[1] + r * angle.sin()]
        }));
    }
    corners.push([middle, y0]);
    corners.push([(middle + BOX_OVERLAP).min(x1 - r).max(middle), y0]);
    corners.dedup();
    corners
}

/// An arrow's start and its tip (normalized): where its drag began, and the point farthest from
/// there, since the barbs turn back toward the start.
pub fn arrow_ends(stroke: &Stroke) -> Option<([f32; 2], [f32; 2])> {
    if stroke.tool != Tool::Arrow {
        return None;
    }
    let points = stroke.normalized_points();
    let from = *points.first()?;
    let (w, h) = (stroke.fw as f32, stroke.fh as f32);
    let far = |p: &[f32; 2]| ((p[0] - from[0]) * w).hypot((p[1] - from[1]) * h);
    let to = points
        .iter()
        .copied()
        .max_by(|a, b| far(a).total_cmp(&far(b)))?;
    Some((from, to))
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn lerp(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shape's drag from `from` to `to` (0…1) on a 1000 × 500 target.
    fn dragged(tool: Tool, from: [f32; 2], to: [f32; 2]) -> Stroke {
        let mut stroke = Stroke {
            t0: 100,
            t1: 100,
            fw: 1000,
            fh: 500,
            on: None,
            pin: None,
            laser: false,
            tool,
            pts: Vec::new(),
        };
        let scaled = |[x, y]: [f32; 2]| [(x * SCALE as f32) as i32, (y * SCALE as f32) as i32];
        drag(&mut stroke, scaled(from), 100);
        drag(&mut stroke, scaled([0.9, 0.9]), 200);
        drag(&mut stroke, scaled(to), 300);
        stroke
    }

    fn in_points(stroke: &Stroke) -> Vec<[f32; 2]> {
        let (w, h) = (stroke.fw as f32, stroke.fh as f32);
        stroke
            .normalized_points()
            .into_iter()
            .map(|[x, y]| [x * w, y * h])
            .collect()
    }

    #[test]
    fn a_drag_keeps_where_it_began_and_where_it_is() {
        let stroke = dragged(Tool::Box, [0.1, 0.2], [0.5, 0.6]);
        assert_eq!(stroke.pts, [1_000, 2_000, 0, 5_000, 6_000, 200]);
        assert_eq!(stroke.t1, 300);
    }

    #[test]
    fn a_finished_box_goes_round_the_drag_and_closes() {
        let stroke = finished(dragged(Tool::Box, [0.5, 0.6], [0.1, 0.2]));
        let points = in_points(&stroke);
        assert_eq!(
            crate::sketch::bounds(&points).map(f32::round),
            [100.0, 100.0, 500.0, 300.0]
        );
        // It begins mid-way along its top edge, and comes round onto it again.
        assert_eq!(points[0].map(f32::round), [300.0, 100.0]);
        let end = *points.last().unwrap();
        assert!(end[1].round() == 100.0 && end[0] > points[0][0], "{end:?}");
        for pair in points.windows(2) {
            assert!(distance(pair[0], pair[1]) <= STEP + 0.5, "{pair:?}");
        }
        assert!(stroke.pts.as_chunks::<3>().0.iter().all(|p| p[2] == 200));
    }

    #[test]
    fn a_finished_arrow_points_from_its_start_to_where_it_let_go() {
        let stroke = finished(dragged(Tool::Arrow, [0.1, 0.5], [0.6, 0.5]));
        let (from, to) = arrow_ends(&stroke).unwrap();
        assert!(distance(from, [0.1, 0.5]) < 0.001 && distance(to, [0.6, 0.5]) < 0.001);
        // Its barbs sit behind the tip, one either side of the shaft.
        let points = in_points(&stroke);
        let behind = points.iter().filter(|p| p[0] < 595.0 && p[0] > 580.0);
        let (above, below) = behind.fold((false, false), |(a, b), p| {
            (a || p[1] < 245.0, b || p[1] > 255.0)
        });
        assert!(above && below);
        assert_eq!(crate::sketch::shape_of(&stroke), crate::model::Shape::Arrow);
    }

    /// Each shape is its own mark, however soon the next follows: a drag is
    /// whole, unlike pen strokes that make one drawing together.
    #[test]
    fn shapes_drawn_one_after_another_each_mark_on_their_own() {
        let boxed = finished(dragged(Tool::Box, [0.1, 0.2], [0.4, 0.5]));
        let mut arrow = finished(dragged(Tool::Arrow, [0.9, 0.9], [0.5, 0.5]));
        arrow.t0 = boxed.t1 + 100;
        let marks = crate::sketch::marks_of(&[boxed, arrow]);
        let shapes: Vec<_> = marks.iter().map(|mark| mark.shape).collect();
        assert_eq!(
            shapes,
            [crate::model::Shape::Circle, crate::model::Shape::Arrow]
        );
        assert!(
            marks[1]
                .description
                .starts_with("drew an arrow from x 0.90, y 0.90")
        );
    }

    #[test]
    fn a_finished_shape_and_ink_stay_as_they_are() {
        let once = finished(dragged(Tool::Box, [0.1, 0.2], [0.5, 0.6]));
        assert_eq!(finished(once.clone()), once);
        let mut ink = dragged(Tool::Box, [0.1, 0.2], [0.5, 0.6]);
        ink.tool = Tool::Pen;
        assert_eq!(finished(ink.clone()), ink);
    }

    #[test]
    fn a_short_arrow_keeps_its_head_inside_its_shaft() {
        let outline = outline(Tool::Arrow, [0.0, 0.0], [20.0, 0.0]);
        let barbs = outline.iter().filter(|p| p[1].abs() > 0.01);
        assert!(barbs.clone().count() > 0);
        assert!(
            barbs
                .into_iter()
                .all(|p| p[0] >= 20.0 - HEAD_SHARE * 20.0 - 0.01)
        );
    }
}
