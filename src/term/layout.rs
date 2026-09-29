//! Pane height: default 35 % of usable height;
//! at least four rows plus padding; leave 120 pt for central content;
//! when the window is too short, central content wins.

pub const PADDING: f32 = 16.0;
pub const MIN_ROWS: f32 = 4.0;
pub const CENTRAL_RESERVE: f32 = 120.0;
pub const DEFAULT_FRACTION: f32 = 0.35;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

pub fn bounds(available: f32, row_height: f32) -> Bounds {
    Bounds {
        min: MIN_ROWS * row_height + PADDING,
        max: available - CENTRAL_RESERVE,
        default: DEFAULT_FRACTION * available,
    }
}

pub fn clamp(requested: Option<f32>, available: f32, row_height: f32) -> f32 {
    let b = bounds(available, row_height);
    let max = b.max.max(0.0);
    if max < b.min {
        return max; // window too short: preserve central content
    }
    requested.unwrap_or(b.default).clamp(b.min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_follow_the_spec_numbers() {
        let b = bounds(700.0, 20.0);
        assert_eq!(b.min, 4.0 * 20.0 + 16.0);
        assert_eq!(b.max, 700.0 - 120.0);
        assert_eq!(b.default, 0.35 * 700.0);
    }

    #[test]
    fn missing_or_out_of_range_requests_clamp() {
        assert_eq!(clamp(None, 700.0, 20.0), 245.0);
        assert_eq!(clamp(Some(10.0), 700.0, 20.0), 96.0);
        assert_eq!(clamp(Some(5000.0), 700.0, 20.0), 580.0);
        assert_eq!(clamp(Some(300.0), 700.0, 20.0), 300.0);
    }

    #[test]
    fn short_window_lets_central_content_win() {
        // available 150: max = 30, min = 96 → central wins → 30.
        assert_eq!(clamp(Some(300.0), 150.0, 20.0), 30.0);
        // available 100: max would be negative → 0, never negative.
        assert_eq!(clamp(Some(300.0), 100.0, 20.0), 0.0);
    }
}
