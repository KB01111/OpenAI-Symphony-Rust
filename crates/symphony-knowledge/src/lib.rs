//! Repository Knowledge Base: live worktrees, a deduplicated commit timeline, a code graph built by
//! external engines (Graphify, optionally Serena) and the mapping from a commit onto that graph.
//!
//! This crate reads; it never writes to a user's repository or worktree. Engines run only on
//! Symphony-owned snapshot directories (see [`snapshot`]).

pub mod analysis;
pub mod engine;
pub mod git;
pub mod graph;
pub mod impact;
pub mod model;
pub mod serena;
pub mod service;
pub mod snapshot;
pub mod store;
pub mod timeline;
pub mod watch;
