//! Miscellaneous helpers shared across modules.

/// Clamp a value between lower and upper bounds.
pub fn clamp<T: Ord>(value: T, min: T, max: T) -> T {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}
