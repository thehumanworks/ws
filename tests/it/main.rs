//! Integration tests for `ws`, compiled as one crate so the mock server in
//! `common` is shared.

#![expect(
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test support code fails loudly by design"
)]

mod cli;
mod client;
mod common;
mod lean_vectors;
