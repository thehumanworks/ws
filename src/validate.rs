//! Local validation of everything sent to Cloudflare.
//!
//! Requests are billed, so invalid input is rejected before any network call.
//! The numeric bounds here are mirrored by the Lean model in `lean/`.

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

#[cfg(test)]
mod tests {
    use super::*;

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
