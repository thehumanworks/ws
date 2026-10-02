//! The web search providers Cloudflare exposes through AI Gateway.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A web search provider selectable in the `provider` request field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// Ceramic.ai — Cloudflare's default provider.
    #[default]
    Ceramic,
    /// Exa.
    Exa,
    /// Linkup.
    Linkup,
}

impl Provider {
    /// Every provider, in the order Cloudflare documents them.
    pub const ALL: [Self; 3] = [Self::Ceramic, Self::Exa, Self::Linkup];

    /// The wire name sent to Cloudflare.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ceramic => "ceramic",
            Self::Exa => "exa",
            Self::Linkup => "linkup",
        }
    }

    /// List price in US dollars per 1,000 searches (documentation snapshot of 2026-10-02).
    #[must_use]
    pub const fn price_per_thousand_usd(self) -> &'static str {
        match self {
            Self::Ceramic => "0.25",
            Self::Exa => "7.00",
            Self::Linkup => "5.00",
        }
    }

    /// One-line description of how the provider behaves behind this API.
    #[must_use]
    pub const fn summary(self) -> &'static str {
        match self {
            Self::Ceramic => "independent index of 40B+ pages; long snippets (up to 8,000 chars)",
            Self::Exa => "\"auto\" search type; highlights returned as the snippet",
            Self::Linkup => "\"fast\" search depth; raw results, no generated answer",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Returned when a string names no known provider.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown provider '{0}' (expected one of: ceramic, exa, linkup)")]
pub struct UnknownProvider(pub String);

impl FromStr for Provider {
    type Err = UnknownProvider;

    /// Parses a provider name, ignoring ASCII case.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|p| p.as_str().eq_ignore_ascii_case(s))
            .ok_or_else(|| UnknownProvider(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_ceramic() {
        assert_eq!(Provider::default(), Provider::Ceramic);
    }

    #[test]
    fn parse_round_trips_every_provider() {
        for p in Provider::ALL {
            assert_eq!(p.as_str().parse::<Provider>(), Ok(p));
            assert_eq!(p.to_string(), p.as_str());
        }
    }

    #[test]
    fn parse_ignores_ascii_case() {
        assert_eq!("EXA".parse::<Provider>(), Ok(Provider::Exa));
        assert_eq!("LinkUp".parse::<Provider>(), Ok(Provider::Linkup));
    }

    #[test]
    fn parse_rejects_unknown_names() {
        for bad in ["", "google", " exa", "exa ", "ceramic.ai"] {
            assert_eq!(
                bad.parse::<Provider>(),
                Err(UnknownProvider(bad.to_owned()))
            );
        }
    }

    #[test]
    fn serializes_as_lowercase_wire_name() {
        for p in Provider::ALL {
            assert_eq!(
                serde_json::to_string(&p).unwrap(),
                format!("\"{}\"", p.as_str())
            );
        }
    }

    #[test]
    fn names_are_distinct() {
        let names: std::collections::HashSet<_> =
            Provider::ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(names.len(), Provider::ALL.len());
    }
}
