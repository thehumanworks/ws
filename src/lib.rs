//! `ws` — web search from the terminal through Cloudflare's AI Gateway Web
//! Search API.
//!
//! The binary is a thin shell around [`cli::run`]; every side effect
//! (arguments, environment, output streams) is injected so the whole program
//! can be exercised from tests.

pub mod cli;
pub mod client;
pub mod config;
pub mod error;
pub mod output;
pub mod provider;
pub mod validate;
