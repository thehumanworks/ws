//! Local validation of everything sent to Cloudflare.
//!
//! Requests are billed, so invalid input is rejected before any network call.
//! Page URLs for `ws fetch` are checked here too, so a typo never leaves the machine.
//! The bounds and rules here are mirrored by the Lean model in `lean/`.

use std::net::IpAddr;

/// Maximum query length, in Unicode scalar values (not bytes).
pub const QUERY_MAX_CHARS: usize = 1024;
/// Smallest accepted result limit.
pub const LIMIT_MIN: u64 = 1;
/// Largest accepted result limit.
pub const LIMIT_MAX: u64 = 10;
/// Limit used when none is configured (Cloudflare's own default).
pub const DEFAULT_LIMIT: u8 = 10;
/// Maximum length of a BYOK alias or gateway ID.
pub const NAME_MAX_LEN: usize = 64;
/// Smallest accepted request timeout, in seconds.
pub const TIMEOUT_MIN_SECS: u64 = 1;
/// Largest accepted request timeout, in seconds.
pub const TIMEOUT_MAX_SECS: u64 = 300;
/// Maximum length of a page URL, in characters.
pub const URL_MAX_CHARS: usize = 2048;

/// Why a value was rejected locally.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    /// The query was empty or contained only whitespace.
    #[error("query must not be empty or whitespace-only")]
    EmptyQuery,
    /// The query exceeded [`QUERY_MAX_CHARS`].
    #[error("query is {actual} characters; the maximum is {max}", max = QUERY_MAX_CHARS)]
    QueryTooLong {
        /// Number of characters in the rejected query.
        actual: usize,
    },
    /// The limit was outside [`LIMIT_MIN`]..=[`LIMIT_MAX`].
    #[error("limit must be between {min} and {max}, got {actual}", min = LIMIT_MIN, max = LIMIT_MAX)]
    LimitOutOfRange {
        /// The rejected limit.
        actual: u64,
    },
    /// The BYOK alias did not match `^[A-Za-z0-9_-]{1,64}$`.
    #[error("BYOK alias must be 1-64 characters from A-Z, a-z, 0-9, '_' and '-'")]
    InvalidAlias,
    /// The gateway ID was empty, too long, or contained whitespace/control characters.
    #[error("gateway ID must be 1-64 visible characters without whitespace")]
    InvalidGatewayId,
    /// The account ID was not 32 hexadecimal characters.
    #[error("Cloudflare account ID must be 32 hexadecimal characters")]
    InvalidAccountId,
    /// The API token was empty or contained whitespace/control characters.
    #[error("Cloudflare API token must be non-empty without whitespace or control characters")]
    InvalidToken,
    /// The timeout was outside [`TIMEOUT_MIN_SECS`]..=[`TIMEOUT_MAX_SECS`].
    #[error("timeout must be between {min} and {max} seconds, got {actual}", min = TIMEOUT_MIN_SECS, max = TIMEOUT_MAX_SECS)]
    TimeoutOutOfRange {
        /// The rejected timeout in seconds.
        actual: u64,
    },
    /// The page URL was not an absolute `http://` or `https://` URL with a host.
    #[error("URL must start with http:// or https:// and name a host")]
    UrlNotHttp,
    /// The page URL contained whitespace or control characters.
    #[error("URL must not contain whitespace or control characters")]
    UrlHasWhitespace,
    /// The page URL exceeded [`URL_MAX_CHARS`].
    #[error("URL is {actual} characters; the maximum is {max}", max = URL_MAX_CHARS)]
    UrlTooLong {
        /// Number of characters in the rejected URL.
        actual: usize,
    },
}

/// Whether a query of `chars` characters is within the API's length bounds.
#[must_use]
pub const fn query_len_ok(chars: usize) -> bool {
    1 <= chars && chars <= QUERY_MAX_CHARS
}

/// Whether `limit` is within the API's bounds.
#[must_use]
pub const fn limit_ok(limit: u64) -> bool {
    LIMIT_MIN <= limit && limit <= LIMIT_MAX
}

/// Validates a search query. The query is returned unmodified.
///
/// # Errors
/// [`ValidationError::EmptyQuery`] or [`ValidationError::QueryTooLong`].
pub fn query(query: &str) -> Result<&str, ValidationError> {
    if query.trim().is_empty() {
        return Err(ValidationError::EmptyQuery);
    }
    let actual = query.chars().count();
    if query_len_ok(actual) {
        Ok(query)
    } else {
        Err(ValidationError::QueryTooLong { actual })
    }
}

/// Validates a result limit.
///
/// # Errors
/// [`ValidationError::LimitOutOfRange`] unless `1 <= limit <= 10`.
pub fn limit(limit: u64) -> Result<u8, ValidationError> {
    let out_of_range = ValidationError::LimitOutOfRange { actual: limit };
    if limit_ok(limit) {
        u8::try_from(limit).map_err(|_| out_of_range)
    } else {
        Err(out_of_range)
    }
}

fn is_name(value: &str, allowed: impl Fn(char) -> bool) -> bool {
    let len = value.chars().count();
    (1..=NAME_MAX_LEN).contains(&len) && value.chars().all(allowed)
}

/// Validates a BYOK alias against `^[A-Za-z0-9_-]{1,64}$`.
///
/// # Errors
/// [`ValidationError::InvalidAlias`].
pub fn alias(alias: &str) -> Result<&str, ValidationError> {
    if is_name(alias, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        Ok(alias)
    } else {
        Err(ValidationError::InvalidAlias)
    }
}

/// Validates an AI Gateway ID.
///
/// # Errors
/// [`ValidationError::InvalidGatewayId`].
pub fn gateway_id(id: &str) -> Result<&str, ValidationError> {
    if is_name(id, |c| !c.is_whitespace() && !c.is_control()) {
        Ok(id)
    } else {
        Err(ValidationError::InvalidGatewayId)
    }
}

/// Validates a Cloudflare account ID. It is interpolated into the request
/// path, so anything but 32 hex characters is refused.
///
/// # Errors
/// [`ValidationError::InvalidAccountId`].
pub fn account_id(id: &str) -> Result<&str, ValidationError> {
    if id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(id)
    } else {
        Err(ValidationError::InvalidAccountId)
    }
}

/// Validates a Cloudflare API token enough to be a safe header value.
///
/// # Errors
/// [`ValidationError::InvalidToken`].
pub fn token(token: &str) -> Result<&str, ValidationError> {
    if !token.is_empty() && token.chars().all(|c| c.is_ascii_graphic()) {
        Ok(token)
    } else {
        Err(ValidationError::InvalidToken)
    }
}

/// Validates a request timeout in seconds.
///
/// # Errors
/// [`ValidationError::TimeoutOutOfRange`].
pub const fn timeout_secs(secs: u64) -> Result<u64, ValidationError> {
    if TIMEOUT_MIN_SECS <= secs && secs <= TIMEOUT_MAX_SECS {
        Ok(secs)
    } else {
        Err(ValidationError::TimeoutOutOfRange { actual: secs })
    }
}

/// Validates a page URL for `ws fetch`. The URL is returned unmodified.
///
/// It must be an absolute `http://` or `https://` URL (scheme matched without
/// regard to ASCII case) that names a host and has no whitespace or control
/// characters.
///
/// # Errors
/// [`ValidationError::UrlHasWhitespace`], [`ValidationError::UrlTooLong`] or
/// [`ValidationError::UrlNotHttp`].
pub fn url(url: &str) -> Result<&str, ValidationError> {
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(ValidationError::UrlHasWhitespace);
    }
    let actual = url.chars().count();
    if actual > URL_MAX_CHARS {
        return Err(ValidationError::UrlTooLong { actual });
    }
    let rest = ["http://", "https://"].iter().find_map(|scheme| {
        let (head, rest) = url.split_at_checked(scheme.len())?;
        head.eq_ignore_ascii_case(scheme).then_some(rest)
    });
    match rest {
        Some(rest) if !rest.starts_with(['/', '?', '#', ':', '@']) && !rest.is_empty() => Ok(url),
        _ => Err(ValidationError::UrlNotHttp),
    }
}

/// Whether an IPv4 address is on the public internet.
///
/// Refused: `0.0.0.0/8`, private ranges (`10/8`, `172.16/12`, `192.168/16`),
/// loopback, carrier-grade NAT (`100.64/10`), link-local (`169.254/16`, which
/// holds cloud metadata services), `192.0.0.0/24`, benchmarking (`198.18/15`),
/// multicast and everything above. Mirrored by `ipv4Public` in the Lean model.
#[must_use]
pub fn ipv4_is_public(octets: [u8; 4]) -> bool {
    let [a, b, c, _] = octets;
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (b == 18 || b == 19))
        || a >= 224)
}

/// Whether an address is on the public internet.
///
/// IPv4 follows [`ipv4_is_public`]; IPv6 refuses loopback, the unspecified
/// address, unique-local, link-local and multicast addresses, and applies the
/// IPv4 rule to addresses that embed one (mapped, compatible, NAT64).
#[must_use]
pub fn ip_is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => ipv4_is_public(v4.octets()),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4() {
                return ipv4_is_public(v4.octets());
            }
            if let [0x64, 0xff9b, 0, 0, 0, 0, high, low] = v6.segments() {
                let ([a, b], [c, d]) = (high.to_be_bytes(), low.to_be_bytes());
                return ipv4_is_public([a, b, c, d]);
            }
            !(v6.is_multicast() || v6.is_unique_local() || v6.is_unicast_link_local())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_and_special_ipv4_ranges_are_not_public() {
        for public in [
            [1, 1, 1, 1],
            [8, 8, 8, 8],
            [93, 184, 215, 14],
            [100, 63, 255, 255],
            [100, 128, 0, 0],
            [172, 15, 255, 255],
            [172, 32, 0, 0],
            [192, 0, 1, 1],
            [192, 167, 1, 1],
            [198, 17, 0, 1],
            [198, 20, 0, 1],
            [223, 255, 255, 255],
        ] {
            assert!(ipv4_is_public(public), "{public:?}");
        }
        for private in [
            [0, 0, 0, 0],
            [0, 1, 2, 3],
            [10, 0, 0, 1],
            [100, 64, 0, 1],
            [100, 127, 255, 255],
            [127, 0, 0, 1],
            [127, 255, 255, 255],
            [169, 254, 169, 254],
            [172, 16, 0, 1],
            [172, 31, 255, 255],
            [192, 0, 0, 1],
            [192, 168, 1, 1],
            [198, 18, 0, 1],
            [198, 19, 255, 255],
            [224, 0, 0, 1],
            [240, 0, 0, 1],
            [255, 255, 255, 255],
        ] {
            assert!(!ipv4_is_public(private), "{private:?}");
        }
    }

    #[test]
    fn ipv6_special_ranges_and_embedded_ipv4_are_not_public() {
        for public in [
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
            "64:ff9b::808:808",
        ] {
            assert!(ip_is_public(public.parse().unwrap()), "{public}");
        }
        for private in [
            "::",
            "::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
            "::192.168.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b::a00:1",
        ] {
            assert!(!ip_is_public(private.parse().unwrap()), "{private}");
        }
        assert!(ip_is_public("8.8.8.8".parse().unwrap()));
        assert!(!ip_is_public("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn url_must_be_absolute_http_or_https() {
        for ok in [
            "https://example.com",
            "http://example.com/a?b=c#d",
            "HTTPS://Example.com/",
            "http://127.0.0.1:8080/x",
            "https://[::1]/",
            "https://exämple.com/ü",
        ] {
            assert_eq!(url(ok), Ok(ok));
        }
        for bad in [
            "",
            "example.com",
            "ftp://example.com",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,hi",
            "http://",
            "https:///path",
            "http://?q",
            "http:example.com",
            "//example.com",
            "é",
        ] {
            assert_eq!(url(bad), Err(ValidationError::UrlNotHttp), "{bad}");
        }
    }

    #[test]
    fn url_rejects_whitespace_controls_and_excess_length() {
        for bad in [
            "https://example.com/a b",
            " https://example.com",
            "https://example.com/\r\nHost: evil",
            "https://example.com/\u{1b}[2J",
        ] {
            assert_eq!(url(bad), Err(ValidationError::UrlHasWhitespace), "{bad}");
        }
        let base = "https://example.com/";
        let max = format!("{base}{}", "a".repeat(URL_MAX_CHARS - base.len()));
        assert_eq!(url(&max), Ok(max.as_str()));
        let over = format!("{max}a");
        assert_eq!(
            url(&over),
            Err(ValidationError::UrlTooLong {
                actual: URL_MAX_CHARS + 1
            })
        );
    }

    #[test]
    fn query_length_boundaries() {
        assert_eq!(query(""), Err(ValidationError::EmptyQuery));
        assert_eq!(query(" \t\n"), Err(ValidationError::EmptyQuery));
        assert_eq!(query("a"), Ok("a"));
        let max = "a".repeat(1024);
        assert_eq!(query(&max), Ok(max.as_str()));
        let over = "a".repeat(1025);
        assert_eq!(
            query(&over),
            Err(ValidationError::QueryTooLong { actual: 1025 })
        );
    }

    #[test]
    fn query_counts_characters_not_bytes() {
        // 1024 four-byte scalars = 4096 bytes: valid by characters.
        let emoji = "🦀".repeat(1024);
        assert_eq!(emoji.len(), 4096);
        assert!(query(&emoji).is_ok());
        let over = "é".repeat(1025);
        assert_eq!(
            query(&over),
            Err(ValidationError::QueryTooLong { actual: 1025 })
        );
    }

    #[test]
    fn query_is_not_rewritten() {
        assert_eq!(query("  spaced  out "), Ok("  spaced  out "));
    }

    #[test]
    fn limit_boundaries() {
        assert_eq!(
            limit(0),
            Err(ValidationError::LimitOutOfRange { actual: 0 })
        );
        assert_eq!(limit(1), Ok(1));
        assert_eq!(limit(10), Ok(10));
        assert_eq!(
            limit(11),
            Err(ValidationError::LimitOutOfRange { actual: 11 })
        );
        assert_eq!(
            limit(u64::MAX),
            Err(ValidationError::LimitOutOfRange { actual: u64::MAX })
        );
        assert!(limit_ok(u64::from(DEFAULT_LIMIT)));
    }

    #[test]
    fn alias_pattern() {
        for ok in ["a", "default", "My_Key-2", &"x".repeat(64)] {
            assert_eq!(alias(ok), Ok(ok));
        }
        for bad in ["", "has space", "dot.ted", "é", "a/b", &"x".repeat(65)] {
            assert_eq!(alias(bad), Err(ValidationError::InvalidAlias));
        }
    }

    #[test]
    fn gateway_id_rules() {
        assert_eq!(gateway_id("default"), Ok("default"));
        assert_eq!(gateway_id("my-gateway_1"), Ok("my-gateway_1"));
        for bad in ["", "two words", "tab\tbed", "nl\n", &"g".repeat(65)] {
            assert_eq!(gateway_id(bad), Err(ValidationError::InvalidGatewayId));
        }
    }

    #[test]
    fn account_id_must_be_32_hex() {
        let good = "0123456789abcdefABCDEF0123456789";
        assert_eq!(account_id(good), Ok(good));
        for bad in [
            "",
            "abc",
            "../../zones/0123456789abcdef01234567",
            &"g".repeat(32),
            &"a".repeat(33),
        ] {
            assert_eq!(account_id(bad), Err(ValidationError::InvalidAccountId));
        }
    }

    #[test]
    fn token_must_be_header_safe() {
        assert_eq!(token("abc-DEF_123"), Ok("abc-DEF_123"));
        for bad in ["", "has space", "line\r\nbreak", "tab\t"] {
            assert_eq!(token(bad), Err(ValidationError::InvalidToken));
        }
    }

    #[test]
    fn timeout_boundaries() {
        assert!(timeout_secs(0).is_err());
        assert_eq!(timeout_secs(1), Ok(1));
        assert_eq!(timeout_secs(300), Ok(300));
        assert_eq!(
            timeout_secs(301),
            Err(ValidationError::TimeoutOutOfRange { actual: 301 })
        );
    }
}
