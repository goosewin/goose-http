//! Date and time utilities for HTTP date header formatting.
//!
//! Provides conversions to and from IMF-fixdate as mandated by RFC 9110.

use std::time::{SystemTime, UNIX_EPOCH};

/// Format the provided time using the IMF-fixdate representation.
pub fn imf_fixdate(time: SystemTime) -> String {
    httpdate::fmt_http_date(time)
}

/// Parse an HTTP date into a `SystemTime` value.
pub fn parse_http_date(value: &str) -> Option<SystemTime> {
    httpdate::parse_http_date(value).ok()
}

/// Return the current time formatted as IMF-fixdate.
pub fn now() -> String {
    imf_fixdate(SystemTime::now())
}

/// Helper returning seconds since the Unix epoch.
pub fn epoch_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_http_date() {
        let original = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        let formatted = imf_fixdate(original);
        let parsed = parse_http_date(&formatted).expect("should parse http date");
        assert_eq!(epoch_seconds(parsed), epoch_seconds(original));
    }
}
