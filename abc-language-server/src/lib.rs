// Copyright 2026 Maurice S. Barnum
// SPDX-License-Identifier: Apache-2.0

//! Library API for the ABC language server.
//!
//! The binary in `src/main.rs` is the thin LSP front-end; the bulk of
//! the document model, analysis, and configuration lives in this crate
//! so integration tests (and any future embedder) can drive the same
//! code paths without spinning up the LSP harness.

// Public surface kept narrow on purpose: integration tests reach in
// through `config` and `document`. The other modules remain
// crate-internal until there is a concrete reason to expose them.
pub mod config;
pub mod document;

mod analysis;
mod backend;
mod position;

pub use analysis::Analysis;
pub use backend::Backend;
pub use position::LineIndex;
