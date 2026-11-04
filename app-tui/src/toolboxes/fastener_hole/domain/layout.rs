//! Geometric web checks. A positive web prevents breakout or overlap; strength and code limits are separate.

use super::tolerance::TolerancedValue;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WebCheck {
    pub nominal: f64,
    pub minimum: f64,
    pub maximum: f64,
}

impl WebCheck {
    pub fn clear(self) -> bool { self.minimum > 0.0 }
}

/// `edge` is hole-centre to free edge; `diameter` includes any countersink at the surface.
pub fn edge_web(edge: TolerancedValue, diameter: TolerancedValue) -> WebCheck {
    WebCheck { nominal: edge.nominal - diameter.nominal / 2.0, minimum: edge.min - diameter.max / 2.0, maximum: edge.max - diameter.min / 2.0 }
}

/// `pitch` is centre-to-centre spacing between two holes.
pub fn pitch_web(pitch: TolerancedValue, first: TolerancedValue, second: TolerancedValue) -> WebCheck {
    WebCheck { nominal: pitch.nominal - (first.nominal + second.nominal) / 2.0, minimum: pitch.min - (first.max + second.max) / 2.0, maximum: pitch.max - (first.min + second.min) / 2.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adverse_tolerance_corners_govern_both_webs() {
        let gap = TolerancedValue::from_nominal_tol(0.50, 0.02, 0.03).unwrap();
        let first = TolerancedValue::from_nominal_tol(0.50, 0.02, 0.04).unwrap();
        let second = TolerancedValue::from_nominal_tol(0.52, 0.01, 0.03).unwrap();
        let edge = edge_web(gap, first);
        assert!((edge.minimum - 0.21).abs() < 1e-12);
        assert!((edge.maximum - 0.29).abs() < 1e-12);
        let pitch = pitch_web(gap, first, second);
        assert!((pitch.minimum + 0.065).abs() < 1e-12);
        assert!(!pitch.clear());
        assert!(!edge_web(TolerancedValue::exact(0.25).unwrap(), TolerancedValue::exact(0.50).unwrap()).clear());
    }
}
