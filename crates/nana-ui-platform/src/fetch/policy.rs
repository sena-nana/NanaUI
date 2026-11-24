use std::collections::BTreeSet;
use std::time::Duration;

#[cfg(feature = "fetch")]
use url::Url;

use super::{
    DEFAULT_FETCH_BODY_LIMIT, DEFAULT_FETCH_REDIRECTS, DEFAULT_FETCH_TIMEOUT,
    DEFAULT_FETCH_WORKERS, FetchError, FetchErrorKind,
};

/// Security and resource limits applied by the host before every network hop.
///
/// One policy covers all HTTP(S) egress, including the engine's own resource
/// loads (`url(...)` images).
///
/// Origins use URL origin serialization (`scheme://host[:port]`) and are
/// matched exactly. The default policy authorizes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPolicy {
    allowed_origins: BTreeSet<String>,
    pub timeout: Duration,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub max_redirects: usize,
    pub worker_count: usize,
}

impl Default for FetchPolicy {
    fn default() -> Self {
        Self {
            allowed_origins: BTreeSet::new(),
            timeout: DEFAULT_FETCH_TIMEOUT,
            max_request_bytes: DEFAULT_FETCH_BODY_LIMIT,
            max_response_bytes: DEFAULT_FETCH_BODY_LIMIT,
            max_redirects: DEFAULT_FETCH_REDIRECTS,
            worker_count: DEFAULT_FETCH_WORKERS,
        }
    }
}

impl FetchPolicy {
    pub fn allow_origin(&mut self, origin: &str) -> Result<&mut Self, FetchError> {
        let serialized = parse_http_origin(origin, true)?;
        self.allowed_origins.insert(serialized);
        Ok(self)
    }

    pub fn with_allowed_origin(mut self, origin: &str) -> Result<Self, FetchError> {
        self.allow_origin(origin)?;
        Ok(self)
    }

    pub fn allowed_origins(&self) -> impl Iterator<Item = &str> {
        self.allowed_origins.iter().map(String::as_str)
    }

    pub fn authorize_url(&self, url: &str) -> Result<(), FetchError> {
        let origin = parse_http_origin(url, false)?;
        self.authorize_normalized_origin(&origin)
    }

    #[cfg(feature = "fetch")]
    pub fn authorize(&self, url: &Url) -> Result<(), FetchError> {
        let origin = exact_origin(url)?;
        self.authorize_normalized_origin(&origin)
    }

    fn authorize_normalized_origin(&self, origin: &str) -> Result<(), FetchError> {
        if self.allowed_origins.contains(origin) {
            Ok(())
        } else {
            Err(FetchError::new(
                FetchErrorKind::Policy,
                format!("fetch origin `{origin}` is not authorized by the host"),
            ))
        }
    }
}

fn parse_http_origin(input: &str, origin_only: bool) -> Result<String, FetchError> {
    let invalid = |msg: String| FetchError::new(FetchErrorKind::InvalidRequest, msg);
    let trimmed = input.trim_matches(|c: char| c.is_ascii_control());
    let Some(colon) = trimmed.find(':') else {
        return Err(invalid(format!(
            "invalid fetch origin `{input}`: relative URL without a base"
        )));
    };
    let scheme_raw = &trimmed[..colon];
    if scheme_raw.is_empty()
        || !scheme_raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    {
        return Err(invalid(format!(
            "invalid fetch origin `{input}`: invalid scheme"
        )));
    }
    let scheme = scheme_raw.to_ascii_lowercase();
    let rest = &trimmed[colon + 1..];
    if !rest.starts_with("//") {
        if !matches!(scheme.as_str(), "http" | "https") {
            return Err(FetchError::new(
                FetchErrorKind::Policy,
                format!("fetch only supports HTTP(S) origins: `{input}`"),
            ));
        }
        return Err(invalid(format!(
            "invalid fetch origin `{input}`: missing `//`"
        )));
    }
    let after_slashes = &rest[2..];
    let split_at = after_slashes
        .find(['/', '?', '#'])
        .unwrap_or(after_slashes.len());
    let authority = &after_slashes[..split_at];
    let suffix = &after_slashes[split_at..];
    if origin_only && !suffix.is_empty() && suffix != "/" {
        return Err(invalid(format!(
            "fetch policy requires an origin, not a URL path: `{input}`"
        )));
    }
    if !matches!(scheme.as_str(), "http" | "https") {
        return Err(FetchError::new(
            FetchErrorKind::Policy,
            format!("fetch only supports HTTP(S) origins: `{input}`"),
        ));
    }
    if authority.is_empty() {
        return Err(FetchError::new(
            if origin_only {
                FetchErrorKind::InvalidRequest
            } else {
                FetchErrorKind::Policy
            },
            format!("fetch origin `{input}` has an empty host"),
        ));
    }
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, hp)| hp);
    let (host, port) = if host_port.starts_with('[') {
        let Some(end_bracket) = host_port.find(']') else {
            return Err(invalid(format!(
                "invalid fetch origin `{input}`: unclosed IPv6 host"
            )));
        };
        let host = host_port[..=end_bracket].to_ascii_lowercase();
        let rem = &host_port[end_bracket + 1..];
        let port = if rem.is_empty() {
            None
        } else if let Some(p) = rem.strip_prefix(':') {
            if p.is_empty() {
                None
            } else {
                Some(
                    p.parse::<u16>()
                        .map_err(|_| invalid(format!("invalid port in `{input}`")))?,
                )
            }
        } else {
            return Err(invalid(format!("invalid host in `{input}`")));
        };
        (host, port)
    } else {
        match host_port.split_once(':') {
            Some((h, p)) => {
                if h.is_empty()
                    || h.contains(['\\', '[', ']'])
                    || h.bytes().any(|b| b.is_ascii_whitespace())
                {
                    return Err(invalid(format!("invalid host in `{input}`")));
                }
                let port = if p.is_empty() {
                    None
                } else {
                    Some(
                        p.parse::<u16>()
                            .map_err(|_| invalid(format!("invalid port in `{input}`")))?,
                    )
                };
                (h.to_ascii_lowercase(), port)
            }
            None => {
                if host_port.is_empty()
                    || host_port.contains(['\\', '[', ']'])
                    || host_port.bytes().any(|b| b.is_ascii_whitespace())
                {
                    return Err(invalid(format!("invalid host in `{input}`")));
                }
                (host_port.to_ascii_lowercase(), None)
            }
        }
    };
    let default_port = if scheme == "http" { 80 } else { 443 };
    Ok(match port {
        Some(p) if p != default_port => format!("{scheme}://{host}:{p}"),
        _ => format!("{scheme}://{host}"),
    })
}

#[cfg(feature = "fetch")]
pub(super) fn exact_origin(url: &Url) -> Result<String, FetchError> {
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(FetchError::new(
            FetchErrorKind::Policy,
            format!("fetch only supports HTTP(S) origins: `{url}`"),
        ));
    }
    Ok(url.origin().ascii_serialization())
}
