//! Command builders for every Codex CLI subcommand.
//!
//! Each subcommand is a builder struct that implements [`CodexCommand`].
//! Builders accumulate flags via method chaining, then call
//! [`CodexCommand::execute`] with a [`Codex`] client to run.

pub mod apply;
pub mod completion;
pub mod exec;
pub mod features;
pub mod fork;
pub mod login;
pub mod mcp;
pub mod mcp_server;
pub mod raw;
pub mod resume;
pub mod review;
pub mod sandbox;
pub mod version;

use std::future::Future;

use crate::Codex;
use crate::error::Result;

/// Trait implemented by all Codex CLI command builders.
///
/// [`args`](CodexCommand::args) returns the CLI arguments the builder would
/// pass to the `codex` binary. [`execute`](CodexCommand::execute) spawns the
/// process and returns typed output.
pub trait CodexCommand: Send + Sync {
    /// The type returned on success.
    type Output: Send;

    /// Build the argument list for this command.
    fn args(&self) -> Vec<String>;

    /// Execute the command against the given [`Codex`] client.
    fn execute(&self, codex: &Codex) -> impl Future<Output = Result<Self::Output>> + Send;
}
