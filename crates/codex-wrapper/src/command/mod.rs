//! Command builders for every Codex CLI subcommand.
//!
//! Each subcommand is a builder struct that implements [`CodexCommand`].
//! Builders accumulate flags via method chaining, then call
//! [`CodexCommand::execute`] with a [`Codex`] client to run.

pub mod apply;
pub mod completion;
pub mod doctor;
pub mod exec;
pub mod features;
pub mod fork;
pub mod login;
pub mod mcp;
pub mod mcp_server;
pub mod plugin;
pub mod raw;
pub mod resume;
pub mod review;
pub mod sandbox;
pub mod session_mgmt;
pub mod update;
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

    /// Render the exact command line this builder will spawn, quoted for a
    /// POSIX shell.
    ///
    /// Useful for logging a reproduction or checking an invocation before
    /// running it. The client's global args precede the command's own, the
    /// same order the spawn uses, because both go through one assembly
    /// function: the preview cannot drift from what runs.
    ///
    /// ```no_run
    /// use codex_wrapper::{Codex, CodexCommand, ExecCommand};
    ///
    /// # fn example() -> codex_wrapper::Result<()> {
    /// let codex = Codex::builder().build()?;
    /// let cmd = ExecCommand::new("fix the failing tests").ephemeral();
    /// println!("{}", cmd.to_command_string(&codex));
    /// // codex exec --ephemeral 'fix the failing tests'
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// The rendering is for humans. It is faithful to the argv, but the args
    /// are passed to the process directly rather than through a shell, so a
    /// shell is never involved at spawn time.
    fn to_command_string(&self, codex: &Codex) -> String {
        crate::exec::command_string(codex, self.args())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::command::exec::{ExecCommand, ExecResumeCommand};

    /// Run a fake codex that echoes its argv, one argument per line.
    async fn spawned_args(cmd: &impl CodexCommand, codex: &Codex) -> Vec<String> {
        let output = crate::exec::run_codex(codex, cmd.args()).await.unwrap();
        output.stdout.lines().map(str::to_string).collect()
    }

    fn echoing_codex() -> Codex {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake-codex-echo-args.sh");
        Codex::builder()
            .binary("/bin/bash")
            .arg(script.to_str().unwrap())
            .config("model=\"gpt-5\"")
            .build()
            .expect("bash must exist")
    }

    /// The preview is only worth anything if it matches the real spawn. This
    /// compares against the argv a process actually received, rather than
    /// against the assembly function the preview itself calls: a test written
    /// that way would still pass if the spawn path stopped using it.
    ///
    /// Quoting is shared with the implementation here, and covered on its own
    /// in `exec::tests`. What this pins is the part a shared function cannot
    /// prove by itself: that the args reaching the process are the same ones,
    /// in the same order, that the preview claims.
    #[tokio::test]
    async fn preview_matches_the_argv_a_spawn_receives() {
        let codex = echoing_codex();
        let cmd = ExecCommand::new("fix the failing tests").ephemeral();

        let spawned = spawned_args(&cmd, &codex).await;
        let preview = cmd.to_command_string(&codex);

        // The fake is `bash <script>`, so the echoed argv is the preview with
        // the binary and the script path removed from the front.
        let rendered: Vec<String> = spawned
            .iter()
            .map(|a| crate::exec::shell_quote(a))
            .collect();
        assert!(
            preview.ends_with(&rendered.join(" ")),
            "preview {preview:?} does not end with the spawned argv {rendered:?}"
        );
        // The global -c pair from the client is in there, ahead of the args.
        assert_eq!(spawned[0], "-c");
        assert_eq!(spawned[1], "model=\"gpt-5\"");
        assert_eq!(spawned[2], "exec");
    }

    #[test]
    fn preview_puts_global_args_before_the_subcommand() {
        let codex = echoing_codex();
        let preview = ExecCommand::new("hi").ephemeral().to_command_string(&codex);
        assert!(
            preview.contains(r#"-c 'model="gpt-5"' exec"#),
            "globals must precede the subcommand: {preview}"
        );
        assert!(preview.ends_with("--ephemeral hi"), "{preview}");
    }

    #[test]
    fn preview_is_available_on_every_builder() {
        let codex = echoing_codex();
        // Provided on the trait, so a resume builder gets it too.
        let preview = ExecResumeCommand::new().last().to_command_string(&codex);
        assert!(preview.contains("exec resume --last"), "{preview}");
    }
}
