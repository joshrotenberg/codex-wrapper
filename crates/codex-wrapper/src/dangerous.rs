//! Opt-in access to the flags that disable codex's safety controls.
//!
//! `--dangerously-bypass-approvals-and-sandbox` turns off every approval
//! prompt and the sandbox. `--dangerously-bypass-hook-trust` lets configured
//! hooks run without confirmation. Both were plain builder methods, reachable
//! from any chain by autocomplete, by a copied snippet, or by an agent editing
//! a call site. A method name is not a barrier.
//!
//! They now need two things that cannot both happen by accident:
//!
//! 1. A [`DangerousClient`], which only constructs when
//!    `CODEX_WRAPPER_ALLOW_DANGEROUS` is set in the process environment.
//! 2. A call through [`Dangerous`], passing that client, which re-checks the
//!    variable at the point of use.
//!
//! The second check is not redundant. A client built while the variable was
//! set stops working the moment it is unset, so the gate reflects the
//! environment at the moment the bypass is applied rather than whenever the
//! client happened to be created.
//!
//! The name matches the sibling crate's `CLAUDE_WRAPPER_ALLOW_DANGEROUS`.
//!
//! # Example
//!
//! ```
//! use codex_wrapper::{ExecCommand, dangerous::{Dangerous, DangerousClient}};
//!
//! // Without the environment variable, there is no way through.
//! assert!(DangerousClient::new().is_err());
//! ```
//!
//! ```no_run
//! use codex_wrapper::{CodexCommand, ExecCommand};
//! use codex_wrapper::dangerous::{Dangerous, DangerousClient};
//!
//! # async fn example(codex: &codex_wrapper::Codex) -> codex_wrapper::Result<()> {
//! // With CODEX_WRAPPER_ALLOW_DANGEROUS set in the environment:
//! let allow = DangerousClient::new()?;
//! let output = ExecCommand::new("rewrite everything")
//!     .bypass_approvals_and_sandbox(&allow)?
//!     .execute(codex)
//!     .await?;
//! # let _ = output;
//! # Ok(())
//! # }
//! ```

use crate::error::{Error, Result};

/// The environment variable that unlocks the bypass flags.
pub const ALLOW_DANGEROUS_ENV: &str = "CODEX_WRAPPER_ALLOW_DANGEROUS";

/// Proof that bypassing codex's safety controls is permitted here.
///
/// Constructing one requires [`ALLOW_DANGEROUS_ENV`] to be set. Holding one is
/// not enough on its own: every [`Dangerous`] method re-checks.
#[derive(Debug, Clone, Copy)]
pub struct DangerousClient {
    // Keeps the type unconstructible except through `new`.
    _private: (),
}

impl DangerousClient {
    /// `Err(Error::DangerousNotAllowed)` unless [`ALLOW_DANGEROUS_ENV`] is set
    /// to a non-empty value.
    pub fn new() -> Result<Self> {
        allowed(&|key| std::env::var(key).ok())?;
        Ok(Self { _private: () })
    }

    /// A client without the environment check, for testing the second gate.
    ///
    /// Exists so a test can prove that holding a client is not sufficient,
    /// without setting a process-wide variable that would race other tests.
    #[cfg(test)]
    pub(crate) fn unchecked() -> Self {
        Self { _private: () }
    }
}

/// The gate itself, over an injected environment lookup.
///
/// Injected rather than reading the process environment directly so tests can
/// exercise the allowed path without mutating global state, which would make
/// them race each other.
pub(crate) fn allowed(env: &impl Fn(&str) -> Option<String>) -> Result<()> {
    match env(ALLOW_DANGEROUS_ENV) {
        Some(value) if !value.trim().is_empty() => Ok(()),
        _ => Err(Error::DangerousNotAllowed {
            variable: ALLOW_DANGEROUS_ENV,
        }),
    }
}

mod sealed {
    pub trait Sealed {}
}

/// Bypassing codex's safety controls, for the builders that support it.
///
/// Sealed: this exists to gate an existing capability, not to be implemented
/// elsewhere.
pub trait Dangerous: sealed::Sealed + Sized {
    /// Disable every approval prompt and the sandbox
    /// (`--dangerously-bypass-approvals-and-sandbox`).
    ///
    /// The model's shell commands run with the permissions of the calling
    /// process, against the real filesystem, with nothing to contain a
    /// mistake.
    ///
    /// # Errors
    ///
    /// [`Error::DangerousNotAllowed`] if [`ALLOW_DANGEROUS_ENV`] is not set
    /// at the moment of the call.
    fn bypass_approvals_and_sandbox(self, allow: &DangerousClient) -> Result<Self>;

    /// Let configured hooks run without the trust prompt
    /// (`--dangerously-bypass-hook-trust`).
    ///
    /// # Errors
    ///
    /// [`Error::DangerousNotAllowed`] if [`ALLOW_DANGEROUS_ENV`] is not set
    /// at the moment of the call.
    fn bypass_hook_trust(self, allow: &DangerousClient) -> Result<Self>;
}

/// Implement [`Dangerous`] over the crate-internal setters.
///
/// The setters are `pub(crate)`, so this trait is the only way to reach them
/// from outside, and every path through it is gated.
macro_rules! impl_dangerous {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl sealed::Sealed for $ty {}

            impl Dangerous for $ty {
                fn bypass_approvals_and_sandbox(
                    self,
                    _allow: &DangerousClient,
                ) -> Result<Self> {
                    allowed(&|key| std::env::var(key).ok())?;
                    Ok(self.set_bypass_approvals_and_sandbox())
                }

                fn bypass_hook_trust(self, _allow: &DangerousClient) -> Result<Self> {
                    allowed(&|key| std::env::var(key).ok())?;
                    Ok(self.set_bypass_hook_trust())
                }
            }
        )+
    };
}

impl_dangerous!(
    crate::command::exec::ExecCommand,
    crate::command::exec::ExecResumeCommand,
    crate::command::review::ReviewCommand,
    crate::command::fork::ForkCommand,
    crate::command::resume::ResumeCommand,
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_is_closed_by_default() {
        // Not set in this process, and nothing here sets it.
        let err = DangerousClient::new().unwrap_err();
        assert!(matches!(err, Error::DangerousNotAllowed { .. }), "{err:?}");
        assert!(err.to_string().contains(ALLOW_DANGEROUS_ENV));
    }

    #[test]
    fn the_gate_opens_for_a_non_empty_value() {
        assert!(allowed(&|_| Some("1".into())).is_ok());
        assert!(allowed(&|_| Some("anything".into())).is_ok());
    }

    /// An exported-but-empty variable is the shape a shell leaves behind after
    /// `export X=`, and it must not count as permission.
    #[test]
    fn a_blank_value_does_not_open_the_gate() {
        assert!(allowed(&|_| Some(String::new())).is_err());
        assert!(allowed(&|_| Some("   ".into())).is_err());
        assert!(allowed(&|_| None).is_err());
    }

    /// The second gate, and the reason it is not redundant: a client built
    /// while the variable was set must stop working once it is gone.
    #[test]
    fn holding_a_client_is_not_enough() {
        use crate::command::exec::ExecCommand;

        let stale = DangerousClient::unchecked();
        let err = ExecCommand::new("rewrite everything")
            .bypass_approvals_and_sandbox(&stale)
            .unwrap_err();
        assert!(matches!(err, Error::DangerousNotAllowed { .. }), "{err:?}");

        let err = ExecCommand::new("rewrite everything")
            .bypass_hook_trust(&stale)
            .unwrap_err();
        assert!(matches!(err, Error::DangerousNotAllowed { .. }), "{err:?}");
    }

    /// Gating must not have disconnected the flags from the command line.
    #[test]
    fn the_flags_still_reach_argv_once_set() {
        use crate::command::exec::ExecCommand;
        use crate::command::{CodexCommand, review::ReviewCommand};

        let args = ExecCommand::new("x")
            .set_bypass_approvals_and_sandbox()
            .set_bypass_hook_trust()
            .args();
        assert!(
            args.iter()
                .any(|a| a == "--dangerously-bypass-approvals-and-sandbox"),
            "{args:?}"
        );
        assert!(
            args.iter().any(|a| a == "--dangerously-bypass-hook-trust"),
            "{args:?}"
        );

        let args = ReviewCommand::new()
            .uncommitted()
            .set_bypass_approvals_and_sandbox()
            .args();
        assert!(
            args.iter()
                .any(|a| a == "--dangerously-bypass-approvals-and-sandbox"),
            "{args:?}"
        );
    }
}
