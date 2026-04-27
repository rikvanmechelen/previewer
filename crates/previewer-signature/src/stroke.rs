use serde::{Deserialize, Serialize};

/// Stable identifier for a signature in the on-disk library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SignatureId(pub u64);

impl SignatureId {
    pub fn random() -> Self {
        // Use system time as a coarse but monotonic-enough id; collisions are
        // extremely unlikely for human-scale signature creation rates.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        Self(nanos)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StrokePoint {
    pub x: f64,
    pub y: f64,
    /// Pen pressure 0.0..=1.0. Mouse input fills this with 1.0.
    pub pressure: f32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Stroke {
    pub points: Vec<StrokePoint>,
}

impl Stroke {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, p: StrokePoint) {
        self.points.push(p);
    }

    /// Douglas–Peucker simplification with `tolerance` in stroke-coord units.
    ///
    /// Guarantees: output point count ≤ input point count, and is idempotent
    /// (simplifying an already-simplified stroke at the same tolerance is a
    /// fixed point).
    pub fn simplified(&self, tolerance: f64) -> Stroke {
        if self.points.len() < 3 {
            return self.clone();
        }
        let mut out = Vec::with_capacity(self.points.len());
        out.push(self.points[0]);
        douglas_peucker(&self.points, 0, self.points.len() - 1, tolerance, &mut out);
        out.push(*self.points.last().unwrap());
        Stroke { points: out }
    }
}

fn douglas_peucker(
    points: &[StrokePoint],
    first: usize,
    last: usize,
    tolerance: f64,
    out: &mut Vec<StrokePoint>,
) {
    if last <= first + 1 {
        return;
    }
    let p1 = points[first];
    let p2 = points[last];
    let mut max_dist = 0.0;
    let mut max_idx = first;
    for (i, p) in points.iter().enumerate().take(last).skip(first + 1) {
        let d = perpendicular_distance(*p, p1, p2);
        if d > max_dist {
            max_dist = d;
            max_idx = i;
        }
    }
    if max_dist > tolerance {
        douglas_peucker(points, first, max_idx, tolerance, out);
        out.push(points[max_idx]);
        douglas_peucker(points, max_idx, last, tolerance, out);
    }
}

fn perpendicular_distance(p: StrokePoint, a: StrokePoint, b: StrokePoint) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len_sq = dx * dx + dy * dy;
    if len_sq < f64::EPSILON {
        // a == b: distance is just |p - a|.
        let ex = p.x - a.x;
        let ey = p.y - a.y;
        return (ex * ex + ey * ey).sqrt();
    }
    // Distance from point p to line through a,b.
    let num = (dy * p.x - dx * p.y + b.x * a.y - b.y * a.x).abs();
    num / len_sq.sqrt()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignatureKind {
    /// Pen-drawn vector strokes.
    Vector { strokes: Vec<Stroke> },
    /// Imported raster image with alpha channel.
    Raster {
        width: u32,
        height: u32,
        /// RGBA8, row-major, top-left origin. Length = width * height * 4.
        pixels: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signature {
    pub id: SignatureId,
    pub name: String,
    pub kind: SignatureKind,
}
