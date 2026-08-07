//! Which credential the CLI would use, without spawning it.
//!
//! A cheap synchronous pre-flight check for health endpoints and for failing
//! fast with a clear message instead of an opaque non-zero exit. It answers a
//! different question from
//! [`LoginStatusCommand`](crate::LoginStatusCommand): that one asks the CLI
//! whether a stored credential is currently valid, this one asks which
//! credential the CLI would pick. Keep both.
//!
//! Nothing here reads or returns a credential value. Environment variables are
//! reported by name, and stored credentials by mode and presence.
//!
//! # How this was determined
//!
//! Read off `codex-cli` 0.145.0 rather than assumed, using `codex doctor`,
//! which reports its own auth resolution. Each state below is a captured run:
//!
//! | Setup | What the CLI reports |
//! |---|---|
//! | neither | `no Codex credentials were found` |
//! | `auth.json` only | `auth is configured`, `stored auth mode chatgpt` |
//! | env var only | `auth is provided by environment`, `auth mode none` |
//! | both | `mixed auth signals: ChatGPT login plus API key env var; HTTP reachability uses API-key mode` |
//!
//! The last row is the precedence: with both present the environment key is
//! what reaches the API, and the CLI itself flags the combination as a
//! warning. [`AuthStrategy::Mixed`] preserves that rather than silently
//! picking a winner.
//!
//! `codex login status` is not the authority here: it reports only stored
//! logins, and says "Not logged in" when an environment variable would in fact
//! be used.
//!
//! # Example
//!
//! ```no_run
//! use codex_wrapper::auth::{self, AuthStrategy};
//!
//! let status = auth::detect();
//! match &status.strategy {
//!     AuthStrategy::None => eprintln!("run `codex login` first"),
//!     AuthStrategy::Mixed { .. } => eprintln!("both configured; the env key wins"),
//!     other => println!("will authenticate via {other:?}"),
//! }
//! ```

use std::path::{Path, PathBuf};

/// Environment variables the CLI accepts a credential from.
///
/// Taken from the 0.145.0 binary, and confirmed for `OPENAI_API_KEY` with a
/// captured `codex doctor` run reporting `auth env vars present`.
pub const AUTH_ENV_VARS: [&str; 3] = ["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_ACCESS_TOKEN"];

/// Which credential the CLI would use.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthStrategy {
    /// Nothing configured. The CLI reports `no Codex credentials were found`.
    None,

    /// A stored login at `$CODEX_HOME/auth.json`.
    Stored {
        /// The file's `auth_mode`, `chatgpt` and `apikey` being the observed
        /// values. `None` if the file has no such field.
        mode: Option<String>,
    },

    /// One or more auth environment variables are set, and no stored login.
    Environment {
        /// Names only. Values are never read.
        vars: Vec<&'static str>,
    },

    /// Both are present, which the CLI warns about as "mixed auth signals".
    ///
    /// The environment key is what reaches the API.
    Mixed {
        /// Names only.
        vars: Vec<&'static str>,
        /// The stored login's mode, which is not what gets used.
        stored_mode: Option<String>,
    },
}

/// The result of [`detect`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthStatus {
    /// Which credential the CLI would use.
    pub strategy: AuthStrategy,
    /// The resolved `CODEX_HOME`.
    pub codex_home: PathBuf,
    /// Where a stored login would live, whether or not it exists.
    pub auth_file: PathBuf,
}

impl AuthStatus {
    /// Whether the CLI would find any credential at all.
    ///
    /// True does not mean the credential is valid: nothing here contacts the
    /// API. Use [`LoginStatusCommand`](crate::LoginStatusCommand) for that.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.strategy != AuthStrategy::None
    }
}

/// Detect how the CLI would authenticate, from the current environment.
///
/// Honors `CODEX_HOME`, defaulting to `~/.codex`.
#[must_use]
pub fn detect() -> AuthStatus {
    detect_with(|key| std::env::var(key).ok())
}

/// [`detect`], but against an explicit `CODEX_HOME`.
///
/// Environment variables are still read from the process.
#[must_use]
pub fn detect_in(codex_home: impl AsRef<Path>) -> AuthStatus {
    let home = codex_home.as_ref().to_path_buf();
    detect_with(move |key| {
        if key == "CODEX_HOME" {
            return Some(home.to_string_lossy().into_owned());
        }
        std::env::var(key).ok()
    })
}

/// The whole of the resolution, over an injected environment.
///
/// Taking a lookup rather than reading the process environment keeps this
/// testable without mutating global state, which would make the tests race
/// each other.
pub(crate) fn detect_with(env: impl Fn(&str) -> Option<String>) -> AuthStatus {
    let codex_home = crate::codex_home::resolve(&env);
    let auth_file = codex_home.join("auth.json");

    let vars: Vec<&'static str> = AUTH_ENV_VARS
        .iter()
        .copied()
        .filter(|key| env(key).is_some_and(|value| !value.trim().is_empty()))
        .collect();

    let stored = read_stored_mode(&auth_file);

    let strategy = match (vars.is_empty(), stored) {
        (true, None) => AuthStrategy::None,
        (true, Some(mode)) => AuthStrategy::Stored { mode },
        (false, None) => AuthStrategy::Environment { vars },
        (false, Some(stored_mode)) => AuthStrategy::Mixed { vars, stored_mode },
    };

    AuthStatus {
        strategy,
        codex_home,
        auth_file,
    }
}

/// `Some(mode)` when a stored login exists, where the inner `Option` is its
/// `auth_mode` field. `None` when there is no readable credential file.
///
/// An unreadable or malformed file counts as no stored login: the CLI cannot
/// use it either, and reporting it as usable would be worse than reporting
/// nothing.
fn read_stored_mode(auth_file: &Path) -> Option<Option<String>> {
    let contents = std::fs::read_to_string(auth_file).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&contents).ok()?;
    let object = parsed.as_object()?;

    // A file with neither a mode nor any credential field is not a login.
    let has_credential = object.contains_key("tokens") || object.contains_key("OPENAI_API_KEY");
    let mode = object
        .get("auth_mode")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    if mode.is_none() && !has_credential {
        return None;
    }
    Some(mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_auth(dir: &Path, contents: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("auth.json"), contents).unwrap();
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("codex-wrapper-auth-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn with_home(home: &Path, extra: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let home = home.to_path_buf();
        let extra: Vec<(String, String)> = extra
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |key| {
            if key == "CODEX_HOME" {
                return Some(home.to_string_lossy().into_owned());
            }
            extra.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn nothing_configured() {
        let home = temp_dir("none");
        let status = detect_with(with_home(&home, &[]));
        assert_eq!(status.strategy, AuthStrategy::None);
        assert!(!status.is_configured());
        assert_eq!(status.auth_file, home.join("auth.json"));
    }

    /// The shape of a real chatgpt login: the field names come from an actual
    /// `~/.codex/auth.json`, with the values replaced.
    #[test]
    fn a_stored_chatgpt_login() {
        let home = temp_dir("chatgpt");
        write_auth(
            &home,
            r#"{"OPENAI_API_KEY":null,"auth_mode":"chatgpt","last_refresh":"2026-08-06T00:00:00Z","tokens":{"id_token":"x"}}"#,
        );

        let status = detect_with(with_home(&home, &[]));
        assert_eq!(
            status.strategy,
            AuthStrategy::Stored {
                mode: Some("chatgpt".into())
            }
        );
        assert!(status.is_configured());
    }

    #[test]
    fn a_stored_api_key_login() {
        let home = temp_dir("apikey");
        write_auth(
            &home,
            r#"{"OPENAI_API_KEY":"sk-secret","auth_mode":"apikey"}"#,
        );

        let status = detect_with(with_home(&home, &[]));
        assert_eq!(
            status.strategy,
            AuthStrategy::Stored {
                mode: Some("apikey".into())
            }
        );
    }

    /// Nothing may expose the credential itself, including through Debug,
    /// which is where a health endpoint would most easily leak it.
    #[test]
    fn a_credential_value_is_never_exposed() {
        let home = temp_dir("secret");
        write_auth(
            &home,
            r#"{"OPENAI_API_KEY":"sk-super-secret","auth_mode":"apikey"}"#,
        );

        let status = detect_with(with_home(&home, &[("OPENAI_API_KEY", "sk-env-secret")]));
        let rendered = format!("{status:?}");

        assert!(!rendered.contains("sk-super-secret"), "{rendered}");
        assert!(!rendered.contains("sk-env-secret"), "{rendered}");
        // The variable's name is reported, which is the useful part.
        assert!(rendered.contains("OPENAI_API_KEY"), "{rendered}");
    }

    #[test]
    fn each_supported_env_var_is_detected() {
        let home = temp_dir("envvars");
        for var in AUTH_ENV_VARS {
            let status = detect_with(with_home(&home, &[(var, "value")]));
            assert_eq!(
                status.strategy,
                AuthStrategy::Environment { vars: vec![var] },
                "{var} was not detected"
            );
        }
    }

    /// The CLI warns about this combination rather than silently preferring
    /// one, and reports that the environment key is what reaches the API.
    #[test]
    fn both_sources_report_as_mixed() {
        let home = temp_dir("mixed");
        write_auth(
            &home,
            r#"{"auth_mode":"chatgpt","tokens":{"id_token":"x"}}"#,
        );

        let status = detect_with(with_home(&home, &[("OPENAI_API_KEY", "sk-env")]));
        assert_eq!(
            status.strategy,
            AuthStrategy::Mixed {
                vars: vec!["OPENAI_API_KEY"],
                stored_mode: Some("chatgpt".into()),
            }
        );
    }

    #[test]
    fn an_empty_env_var_is_not_a_credential() {
        let home = temp_dir("blank");
        let status = detect_with(with_home(&home, &[("OPENAI_API_KEY", "   ")]));
        assert_eq!(status.strategy, AuthStrategy::None);
    }

    /// A file the CLI cannot use must not read as configured, or a pre-flight
    /// check passes and the run fails anyway.
    #[test]
    fn a_malformed_auth_file_is_not_a_login() {
        let home = temp_dir("malformed");
        write_auth(&home, "not json at all");
        assert_eq!(
            detect_with(with_home(&home, &[])).strategy,
            AuthStrategy::None
        );

        write_auth(&home, r#"{"unrelated":true}"#);
        assert_eq!(
            detect_with(with_home(&home, &[])).strategy,
            AuthStrategy::None
        );
    }

    #[test]
    fn codex_home_defaults_under_the_user_home() {
        let status = detect_with(|key| match key {
            "HOME" => Some("/home/someone".into()),
            _ => None,
        });
        assert_eq!(status.codex_home, PathBuf::from("/home/someone/.codex"));
        assert_eq!(
            status.auth_file,
            PathBuf::from("/home/someone/.codex/auth.json")
        );
    }

    #[test]
    fn an_empty_codex_home_falls_back_to_the_default() {
        let status = detect_with(|key| match key {
            "CODEX_HOME" => Some(String::new()),
            "HOME" => Some("/home/someone".into()),
            _ => None,
        });
        assert_eq!(status.codex_home, PathBuf::from("/home/someone/.codex"));
    }
}
