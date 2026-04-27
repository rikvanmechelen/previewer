//! Property + unit tests for stroke types and simplification.

use previewer_signature::{Stroke, StrokePoint};
use proptest::prelude::*;

fn arb_point() -> impl Strategy<Value = StrokePoint> {
    (-1000.0_f64..1000.0, -1000.0_f64..1000.0, 0.0_f32..=1.0).prop_map(|(x, y, p)| StrokePoint {
        x,
        y,
        pressure: p,
    })
}

fn arb_stroke() -> impl Strategy<Value = Stroke> {
    prop::collection::vec(arb_point(), 0..50).prop_map(|points| Stroke { points })
}

proptest! {
    /// Simplification must never produce more points than the input.
    #[test]
    fn simplification_is_bounded(stroke in arb_stroke()) {
        let simplified = stroke.simplified(1.0);
        prop_assert!(simplified.points.len() <= stroke.points.len());
    }

    /// Simplifying twice with the same tolerance yields the same result as
    /// simplifying once: simplify is idempotent.
    #[test]
    fn simplification_is_idempotent(stroke in arb_stroke()) {
        let once = stroke.simplified(1.0);
        let twice = once.clone().simplified(1.0);
        prop_assert_eq!(once.points.len(), twice.points.len());
        for (a, b) in once.points.iter().zip(twice.points.iter()) {
            prop_assert!((a.x - b.x).abs() < f64::EPSILON);
            prop_assert!((a.y - b.y).abs() < f64::EPSILON);
        }
    }
}

#[test]
fn empty_stroke_simplifies_to_empty() {
    let s = Stroke { points: vec![] };
    assert!(s.simplified(1.0).points.is_empty());
}

#[test]
fn single_point_stroke_simplifies_to_itself() {
    let s = Stroke {
        points: vec![StrokePoint {
            x: 1.0,
            y: 2.0,
            pressure: 0.5,
        }],
    };
    assert_eq!(s.clone().simplified(1.0).points.len(), 1);
}

#[test]
fn collinear_points_collapse_to_endpoints() {
    // 5 collinear points along y=x. With any positive tolerance, the
    // intermediates have zero perpendicular distance from the first-last
    // line and should drop, leaving just first + last.
    let s = Stroke {
        points: (0..5)
            .map(|i| StrokePoint {
                x: i as f64,
                y: i as f64,
                pressure: 1.0,
            })
            .collect(),
    };
    let result = s.simplified(0.1);
    assert_eq!(result.points.len(), 2);
}

#[test]
fn far_off_point_is_preserved() {
    // Three points: (0,0), (5, 100), (10, 0). The middle point is far from
    // the line (0,0)-(10,0). At tolerance 1.0 it must be preserved.
    let s = Stroke {
        points: vec![
            StrokePoint {
                x: 0.0,
                y: 0.0,
                pressure: 1.0,
            },
            StrokePoint {
                x: 5.0,
                y: 100.0,
                pressure: 1.0,
            },
            StrokePoint {
                x: 10.0,
                y: 0.0,
                pressure: 1.0,
            },
        ],
    };
    let result = s.simplified(1.0);
    assert_eq!(result.points.len(), 3);
}
