//! HTTP caching helpers covering validators, freshness, and revalidation.
//!
//! Implements parsing of `Cache-Control`, derivation of freshness metadata, and
//! helpers for ensuring compliant response headers per RFC 9111.

use std::time::{Duration, SystemTime};

use crate::{
    date,
    headers::{Headers, header_keys},
};

/// Represents parsed Cache-Control directives.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CacheControl {
    pub no_store: bool,
    pub no_cache: bool,
    pub must_revalidate: bool,
    pub max_age: Option<u64>,
    pub other: Vec<String>,
}

/// Represents cache policy metadata derived from headers.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CachePolicy {
    pub control: Option<CacheControl>,
    pub age: Option<u64>,
    pub expires: Option<SystemTime>,
    pub vary: Vec<String>,
}

/// Parse a Cache-Control header into structured directives.
pub fn parse_cache_control(value: &str) -> CacheControl {
    let mut control = CacheControl::default();
    for directive in value.split(',') {
        let directive = directive.trim();
        if directive.is_empty() {
            continue;
        }
        let mut parts = directive.splitn(2, '=');
        let token = parts.next().unwrap().trim().to_ascii_lowercase();
        let param = parts.next().map(|p| p.trim_matches('"'));
        match token.as_str() {
            "no-store" => control.no_store = true,
            "no-cache" => control.no_cache = true,
            "must-revalidate" => control.must_revalidate = true,
            "max-age" => {
                if let Some(value) = param.and_then(|p| p.parse::<u64>().ok()) {
                    control.max_age = Some(value);
                }
            }
            _ => control.other.push(directive.to_owned()),
        }
    }
    control
}

/// Analyse response headers to produce a [`CachePolicy`].
pub fn policy_from_headers(headers: &Headers) -> CachePolicy {
    let control = headers
        .get(header_keys::CACHE_CONTROL)
        .map(parse_cache_control);

    let age = headers
        .get(header_keys::AGE)
        .and_then(|value| value.parse::<u64>().ok());

    let expires = headers
        .get(header_keys::EXPIRES)
        .and_then(date::parse_http_date);

    let vary = headers
        .get(header_keys::VARY)
        .map(|value| value.split(',').map(|v| v.trim().to_string()).collect())
        .unwrap_or_default();

    CachePolicy {
        control,
        age,
        expires,
        vary,
    }
}

/// Compute the freshness lifetime hinted by the headers.
pub fn freshness_lifetime(policy: &CachePolicy, date: Option<SystemTime>) -> Option<Duration> {
    if let Some(control) = &policy.control
        && let Some(max_age) = control.max_age
    {
        return Some(Duration::from_secs(max_age));
    }

    if let (Some(expires), Some(date_value)) = (policy.expires, date)
        && let Ok(delta) = expires.duration_since(date_value)
    {
        return Some(delta);
    }

    None
}

/// Ensure the Age header is set when absent (origin servers should emit Age).
pub fn ensure_age_header(headers: &mut Headers) {
    if !headers.contains(header_keys::AGE) {
        headers.insert(header_keys::AGE, "0");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cache_control_recognises_common_directives() {
        let control = parse_cache_control("max-age=60, must-revalidate, no-store");
        assert_eq!(control.max_age, Some(60));
        assert!(control.must_revalidate);
        assert!(control.no_store);
        assert!(!control.no_cache);
    }

    #[test]
    fn policy_extracts_age_and_vary() {
        let mut headers = Headers::new();
        headers.insert(header_keys::CACHE_CONTROL, "max-age=120");
        headers.insert(header_keys::AGE, "15");
        headers.insert(header_keys::VARY, "Accept-Encoding, Accept-Language");
        let policy = policy_from_headers(&headers);
        assert_eq!(policy.control.unwrap().max_age, Some(120));
        assert_eq!(policy.age, Some(15));
        assert_eq!(policy.vary.len(), 2);
    }
}
