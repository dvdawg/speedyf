//! Affine matrices and axis-aligned boxes in PDF user space.

/// A PDF matrix `[a b c d e f]`: a point maps as
/// `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix(pub [f32; 6]);

impl Matrix {
    pub const IDENTITY: Matrix = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub fn translate(x: f32, y: f32) -> Matrix {
        Matrix([1.0, 0.0, 0.0, 1.0, x, y])
    }

    /// `self` applied first, then `then` - the PDF `cm` order:
    /// `cm M` makes the new CTM `M x CTM`, i.e. `M.then(ctm)`.
    pub fn then(&self, then: &Matrix) -> Matrix {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = then.0;
        Matrix([
            a * a2 + b * c2,
            a * b2 + b * d2,
            c * a2 + d * c2,
            c * b2 + d * d2,
            e * a2 + f * c2 + e2,
            e * b2 + f * d2 + f2,
        ])
    }

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }

    pub fn invert(&self) -> Option<Matrix> {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        let ia = d / det;
        let ib = -b / det;
        let ic = -c / det;
        let id = a / det;
        Some(Matrix([
            ia,
            ib,
            ic,
            id,
            -(e * ia + f * ic),
            -(e * ib + f * id),
        ]))
    }

    /// The bounding box of the rectangle `[x0,y0]-[x1,y1]` mapped by this matrix.
    pub fn map_box(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::bounding(&[
            self.apply(x0, y0),
            self.apply(x1, y0),
            self.apply(x1, y1),
            self.apply(x0, y1),
        ])
    }

    pub fn from_operands(ops: &[lopdf::Object]) -> Option<Matrix> {
        if ops.len() != 6 {
            return None;
        }
        let mut m = [0.0f32; 6];
        for (slot, op) in m.iter_mut().zip(ops) {
            *slot = number(op)?;
        }
        Some(Matrix(m))
    }
}

/// A number operand (integer or real).
pub fn number(o: &lopdf::Object) -> Option<f32> {
    match o {
        lopdf::Object::Integer(i) => Some(*i as f32),
        lopdf::Object::Real(r) => Some(*r),
        _ => None,
    }
}

/// An axis-aligned box, `x0 <= x1`, `y0 <= y1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect {
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
        }
    }

    pub fn bounding(points: &[(f32, f32)]) -> Rect {
        let mut r = Rect {
            x0: f32::INFINITY,
            y0: f32::INFINITY,
            x1: f32::NEG_INFINITY,
            y1: f32::NEG_INFINITY,
        };
        for &(x, y) in points {
            r.x0 = r.x0.min(x);
            r.y0 = r.y0.min(y);
            r.x1 = r.x1.max(x);
            r.y1 = r.y1.max(y);
        }
        r
    }

    pub fn is_valid(&self) -> bool {
        self.x0.is_finite() && self.y0.is_finite() && self.x1.is_finite() && self.y1.is_finite()
    }

    pub fn width(&self) -> f32 {
        (self.x1 - self.x0).max(0.0)
    }

    pub fn height(&self) -> f32 {
        (self.y1 - self.y0).max(0.0)
    }

    pub fn area(&self) -> f32 {
        self.width() * self.height()
    }

    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x0 = self.x0.max(other.x0);
        let y0 = self.y0.max(other.y0);
        let x1 = self.x1.min(other.x1);
        let y1 = self.y1.min(other.y1);
        (x1 > x0 && y1 > y0).then_some(Rect { x0, y0, x1, y1 })
    }

    /// Overlap with positive area.
    pub fn overlaps(&self, other: &Rect) -> bool {
        self.intersection(other).is_some()
    }

    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }

    pub fn contains(&self, other: &Rect) -> bool {
        other.x0 >= self.x0 && other.x1 <= self.x1 && other.y0 >= self.y0 && other.y1 <= self.y1
    }

    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invert_round_trips() {
        let m = Matrix([0.0, 2.0, -3.0, 0.0, 10.0, 20.0]);
        let inv = m.invert().unwrap();
        let (x, y) = m.apply(4.0, 5.0);
        let (bx, by) = inv.apply(x, y);
        assert!((bx - 4.0).abs() < 1e-4 && (by - 5.0).abs() < 1e-4);
    }

    #[test]
    fn then_is_pdf_order() {
        // translate then scale: point (1,0) -> (1+10, 0) -> (22, 0)
        let m = Matrix::translate(10.0, 0.0).then(&Matrix([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]));
        assert_eq!(m.apply(1.0, 0.0), (22.0, 0.0));
    }
}
