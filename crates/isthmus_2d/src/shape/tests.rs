//! Bounds and scaling invariants of the shape constructors and combinators.
use super::*;

/// Declared bounds minus the true extent of the interior, per side: (min, max).
fn slack(shape: impl Sdf) -> (Vec2, Vec2) {
    let (mut min, mut max) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for x in 0..800 {
        for y in 0..800 {
            let point = vec2(x as f32, y as f32) * 0.25 - 100.0;
            if shape.contains(point) {
                (min, max) = (min.min(point), max.max(point));
            }
        }
    }
    let bounds = shape.geometry_bounds();
    (min - bounds.min, bounds.max - max)
}

/// Bounds must never cut the shape off; exact ones must also hug it (to within the sampling grid and thin tips).
fn assert_bounds(name: &str, shape: impl Sdf, exact: bool) {
    let (min, max) = slack(shape);
    let (low, high) = (min.min(max).min_element(), min.max(max).max_element());
    assert!(low >= -0.3, "{name} bounds cut off the shape by {}", -low);
    assert!(!exact || high <= 0.5, "{name} bounds overshoot by {high}");
}

#[test]
fn primitive_bounds_hug_their_shapes_and_combinators_enclose_them() {
    let rect = Rect::new(vec2(-40.0, -20.0), vec2(60.0, 30.0));
    let (a, b) = (Shape::rounded_rect(rect, 12.0), Shape::circle(vec2(30.0, 10.0), 25.0));
    assert_bounds("rounded_rect", a, true);
    assert_bounds("pill", Shape::pill(rect), true);
    assert_bounds("squircle", Shape::side_rounded_rect(rect, Corner { radius: 18.0, roundness: 0.0 }, 9.0), true);
    assert_bounds("circle", b, true);
    assert_bounds("segment", Shape::segment(vec2(-30.0, -10.0), vec2(50.0, 40.0), 10.0), true);
    assert_bounds("arc", Shape::arc(Vec2::ZERO, 40.0, 0.5, 2.0).offset(3.0), false);
    assert_bounds("star", Shape::star(40.0, 16.0), true);
    assert_bounds("hexagon", Shape::hexagon(vec2(5.0, -3.0), 50.0), true);
    assert_bounds("rounded hexagon", Shape::hexagon(vec2(5.0, -3.0), 30.0).offset(15.0), true);
    assert_bounds("offset", a.offset(6.0), true);
    assert_bounds("stroke", a.stroke(8.0), true);
    assert_bounds("scaled", a.scaled(vec2(1.5, 0.7)), true);
    assert_bounds("union", a.union(b), true);
    assert_bounds("intersection", a.intersection(b), true);
    assert_bounds("difference", a.difference(b), false);
    assert_bounds("smooth_union", a.smooth_union(b, 20.0, 1.0), false);
}

#[test]
fn scaling_keeps_the_field_a_distance_and_unions_keep_the_larger_outline() {
    let shape = Shape::rounded_rect(Rect::new(vec2(10.0, -20.0), vec2(90.0, 30.0)), 12.0);
    let uniform = shape.scaled(Vec2::splat(2.0)).distance_at(vec2(200.0, 0.0));
    assert!((uniform - 2.0 * shape.distance_at(vec2(100.0, 0.0))).abs() < 1e-3);
    // Unequal factors may only underestimate, so antialiasing and shadows never overshoot.
    let stretched = shape.scaled(vec2(1.0, 3.0));
    assert!(stretched.distance_at(vec2(150.0, 0.0)) <= shape.distance_at(vec2(150.0, 0.0)) + 1e-3);

    let (small, large) = (Shape::circle(Vec2::ZERO, 5.0), Shape::circle(vec2(20.0, 0.0), 5.0).outlined(12.0));
    for combined in [small.union(large).outline_width(), small.smooth_union(large, 4.0, 1.0).outline_width()] {
        assert_eq!(combined, 12.0);
    }
    assert_eq!(large.intersection(small).outline_width(), 12.0);
}
