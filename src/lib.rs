//! `ws` — runtime-selectable local Lightpanda/Brave or Cloudflare search,
//! rendered page reading, direct HTTP fetching and text-only PNG output.
//!
//! The binary is a thin shell around [`cli::run`]; every side effect
//! (arguments, environment, output streams) is injected so the whole program
//! can be exercised from tests.

pub mod backend;
pub mod body;
pub mod brave;
pub mod browser_asset;
pub mod cli;
pub mod client;
pub mod config;
pub mod error;
pub mod extract;
pub mod fetch;
pub mod lightpanda;
pub mod markdown;
pub mod output;
pub mod provider;
pub mod render;
pub mod validate;
