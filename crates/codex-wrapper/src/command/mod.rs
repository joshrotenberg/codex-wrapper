pub mod exec;
pub mod login;
pub mod mcp;
pub mod raw;
pub mod review;
pub mod version;

use std::future::Future;

use crate::Codex;
use crate::error::Result;

/// Trait implemented by all codex CLI command builders.
pub trait CodexCommand: Send + Sync {
    type Output: Send;

    fn args(&self) -> Vec<String>;

    fn execute(&self, codex: &Codex) -> impl Future<Output = Result<Self::Output>> + Send;
}
