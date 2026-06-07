//! Byte range parsing utilities.
//!
//! Implements structures for representing satisfiable and unsatisfiable byte
//! ranges, following RFC 9110 Section 14.

use std::cmp::{max, min};

use thiserror::Error;

/// Represents a single byte range specification from the Range header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeSpec {
    /// `start-end`
    FromTo { start: u64, end: u64 },
    /// `start-`
    From { start: u64 },
    /// `-suffix-length`
    Suffix { length: u64 },
}

/// Represents a satisfiable inclusive byte range once applied to an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SatisfiableRange {
    pub start: u64,
    pub end: u64,
}

impl SatisfiableRange {
    pub fn len(self) -> u64 {
        self.end - self.start + 1
    }

    pub fn is_empty(self) -> bool {
        self.end < self.start
    }
}

/// Errors that can occur when parsing or applying range headers.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RangeParseError {
    #[error("range unit must be bytes")]
    InvalidUnit,
    #[error("invalid range syntax")]
    InvalidSyntax,
    #[error("range start greater than end")]
    InvalidBounds,
    #[error("range component is not numeric")]
    InvalidNumber,
    #[error("empty range set")]
    Empty,
}

/// Parse a Range header value into byte range specifications.
pub fn parse_range_header(value: &str) -> Result<Vec<RangeSpec>, RangeParseError> {
    let (unit, ranges) = value
        .split_once('=')
        .ok_or(RangeParseError::InvalidSyntax)?;
    if !unit.trim().eq_ignore_ascii_case("bytes") {
        return Err(RangeParseError::InvalidUnit);
    }

    let mut specs = Vec::new();
    for part in ranges.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some(stripped) = part.strip_prefix('-') {
            let length = stripped
                .trim()
                .parse::<u64>()
                .map_err(|_| RangeParseError::InvalidNumber)?;
            specs.push(RangeSpec::Suffix { length });
            continue;
        }

        let mut bounds = part.splitn(2, '-');
        let start = bounds
            .next()
            .ok_or(RangeParseError::InvalidSyntax)?
            .trim()
            .parse::<u64>()
            .map_err(|_| RangeParseError::InvalidNumber)?;

        match bounds.next() {
            Some("") | None => {
                specs.push(RangeSpec::From { start });
            }
            Some(end_str) => {
                let end = end_str
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| RangeParseError::InvalidNumber)?;
                if start > end {
                    return Err(RangeParseError::InvalidBounds);
                }
                specs.push(RangeSpec::FromTo { start, end });
            }
        }
    }

    if specs.is_empty() {
        return Err(RangeParseError::Empty);
    }

    Ok(specs)
}

/// Compute satisfiable ranges given entity length. Returns an empty vector when
/// no ranges overlap with the entity.
pub fn compute_satisfiable_ranges(specs: &[RangeSpec], total_length: u64) -> Vec<SatisfiableRange> {
    if total_length == 0 {
        return Vec::new();
    }

    specs
        .iter()
        .filter_map(|spec| match spec {
            RangeSpec::FromTo { start, end } => {
                if *start >= total_length {
                    None
                } else {
                    let bounded_end = min(*end, total_length - 1);
                    Some(SatisfiableRange {
                        start: *start,
                        end: max(*start, bounded_end),
                    })
                }
            }
            RangeSpec::From { start } => {
                if *start >= total_length {
                    None
                } else {
                    Some(SatisfiableRange {
                        start: *start,
                        end: total_length - 1,
                    })
                }
            }
            RangeSpec::Suffix { length } => {
                let length = min(*length, total_length);
                if length == 0 {
                    None
                } else {
                    Some(SatisfiableRange {
                        start: total_length - length,
                        end: total_length - 1,
                    })
                }
            }
        })
        .collect()
}

/// Format a Content-Range header for a satisfiable range.
pub fn format_content_range(range: SatisfiableRange, total_length: u64) -> String {
    format!("bytes {}-{}/{}", range.start, range.end, total_length)
}

/// Format a Content-Range header for unsatisfied ranges.
pub fn format_unsatisfied_range(total_length: u64) -> String {
    format!("bytes */{}", total_length)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_range() {
        let parsed = parse_range_header("bytes=0-99").unwrap();
        assert_eq!(parsed, vec![RangeSpec::FromTo { start: 0, end: 99 }]);
    }

    #[test]
    fn parse_suffix_range() {
        let parsed = parse_range_header("bytes=-500").unwrap();
        assert_eq!(parsed, vec![RangeSpec::Suffix { length: 500 }]);
    }

    #[test]
    fn compute_satisfiable_from_range() {
        let specs = parse_range_header("bytes=100-").unwrap();
        let ranges = compute_satisfiable_ranges(&specs, 1000);
        assert_eq!(
            ranges,
            vec![SatisfiableRange {
                start: 100,
                end: 999
            }]
        );
    }

    #[test]
    fn unsatisfiable_returns_empty() {
        let specs = parse_range_header("bytes=2000-3000").unwrap();
        let ranges = compute_satisfiable_ranges(&specs, 1024);
        assert!(ranges.is_empty());
    }
}
