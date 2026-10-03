//! Retrieval backends are independent of Cloudflare search providers.

use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// Which runtime performs a search or fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// The bundled local JavaScript browser, using Brave for search.
    Lightpanda,
    /// Cloudflare search, direct HTTP fetch, or opt-in Browser Run.
    Cloudflare,
}

impl Backend {
    /// The default: local on supported Unix systems, Cloudflare elsewhere.
    #[must_use]
    pub const fn platform_default() -> Self {
        if env!("WS_BROWSER_HASH").is_empty() {
            Self::Cloudflare
        } else {
            Self::Lightpanda
        }
    }

    /// Stable CLI/config spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lightpanda => "lightpanda",
            Self::Cloudflare => "cloudflare",
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Backend {
    type Err = crate::error::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "lightpanda" => Ok(Self::Lightpanda),
            "cloudflare" => Ok(Self::Cloudflare),
            _ => Err(crate::error::Error::Config(
                "backend must be lightpanda or cloudflare".to_owned(),
            )),
        }
    }
}
