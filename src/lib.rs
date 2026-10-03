//! `ws` — web search from the terminal through Cloudflare's AI Gateway Web
//! Search API, and plain page fetching (`ws fetch`) to read what a search finds.
//!
//! The binary is a thin shell around [`cli::run`]; every side effect
//! (arguments, environment, output streams) is injected so the whole program
//! can be exercised from tests.

pub mod body;
pub mod cli;
pub mod client;
pub mod config;
pub mod error;
pub mod extract;
pub mod fetch;
pub mod markdown;
pub mod output;
pub mod provider;
pub mod render;
pub mod validate;
