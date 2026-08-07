//! Resolving `CODEX_HOME`, shared by the read-side modules.
//!
//! Both [`crate::auth`] and [`crate::config`] look inside the CLI's home
//! directory, and they have to agree on where it is. They also each sit behind
//! a different feature, so this lives on its own rather than in either.

use std::path::{Path, PathBuf};

/// Resolve the CLI's home directory over an injected environment lookup.
///
/// `CODEX_HOME` when set and non-empty, otherwise `$HOME/.codex`.
///
/// The lookup is injected rather than read from the process so callers and
/// tests can supply their own environment without mutating global state.
pub(crate) fn resolve(env: &impl Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(home) = env("CODEX_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(home);
    }
    env("HOME").filter(|home| !home.is_empty()).map_or_else(
        || PathBuf::from(".codex"),
        |home| Path::new(&home).join(".codex"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_home_wins_when_set() {
        let home = resolve(&|key| match key {
            "CODEX_HOME" => Some("/somewhere/else".into()),
            "HOME" => Some("/home/someone".into()),
            _ => None,
        });
        assert_eq!(home, PathBuf::from("/somewhere/else"));
    }

    #[test]
    fn falls_back_to_the_user_home() {
        let home = resolve(&|key| match key {
            "HOME" => Some("/home/someone".into()),
            _ => None,
        });
        assert_eq!(home, PathBuf::from("/home/someone/.codex"));
    }

    /// An empty value is not a path. Treating it as one would resolve the home
    /// to the filesystem root.
    #[test]
    fn an_empty_codex_home_is_ignored() {
        let home = resolve(&|key| match key {
            "CODEX_HOME" => Some(String::new()),
            "HOME" => Some("/home/someone".into()),
            _ => None,
        });
        assert_eq!(home, PathBuf::from("/home/someone/.codex"));
    }

    #[test]
    fn a_relative_default_when_nothing_is_set() {
        assert_eq!(resolve(&|_| None), PathBuf::from(".codex"));
    }
}
