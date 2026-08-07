//! Read-side access to `$CODEX_HOME/config.toml`.
//!
//! Requires the `config` feature, which is off by default: it pulls in a TOML
//! parser, and a caller that never reads config should not pay for one.
//!
//! This matters more than it used to. Several `exec` options moved from flags
//! to config keys in 0.145.0 (`approval_policy`, `web_search`), so config is
//! where some behavior is now decided, and a host reporting or overriding
//! effective settings would otherwise parse the file itself.
//!
//! # What is typed and what is not
//!
//! Only the keys this wrapper has a reason to know about are typed. Everything
//! else stays in [`CodexConfig::raw`] as parsed TOML. Modelling the whole file
//! would mean tracking a schema that changes every release, and a key this
//! crate cannot name is not a key it should hide.
//!
//! # Profiles are files, not a table
//!
//! Verified against `codex-cli` 0.145.0: `--profile <name>` layers
//! `$CODEX_HOME/<name>.config.toml` over the base config, so the available
//! profiles are the `*.config.toml` files in the home directory.
//!
//! A `[profiles]` table in `config.toml` is the legacy mechanism. The CLI now
//! refuses to write one, reporting that it "contains legacy config profile
//! tables and can no longer be written". Any such table is reported separately
//! as [`CodexConfig::legacy_profiles`] rather than mixed in with the real ones.
//!
//! # Example
//!
//! ```no_run
//! # fn example() -> codex_wrapper::Result<()> {
//! if let Some(config) = codex_wrapper::config::load()? {
//!     println!("model:    {:?}", config.model);
//!     println!("profiles: {:?}", config.profiles);
//! }
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The parsed contents of `config.toml`, plus the profiles beside it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CodexConfig {
    /// The file this was read from.
    pub path: PathBuf,

    /// Default model (`model`).
    pub model: Option<String>,
    /// When the model asks for approval (`approval_policy`).
    pub approval_policy: Option<String>,
    /// Sandbox policy (`sandbox_mode`).
    pub sandbox_mode: Option<String>,
    /// Web search mode (`web_search`).
    pub web_search: Option<String>,

    /// Feature flags under `[features]`, for the ones with boolean values.
    pub features: BTreeMap<String, bool>,

    /// Per-directory trust, from `[projects."<path>"]` tables.
    ///
    /// This is what decides the `Not inside a trusted directory` refusal that
    /// [`crate::Error::NotTrustedDirectory`] classifies.
    pub project_trust: BTreeMap<String, String>,

    /// Profile names, from the `<name>.config.toml` files beside this one.
    pub profiles: Vec<String>,

    /// Names from a legacy `[profiles]` table, if the file still has one.
    ///
    /// The CLI no longer writes these. Present so an old config is visible
    /// rather than silently ignored.
    pub legacy_profiles: Vec<String>,

    /// Everything in the file, including the keys typed above.
    pub raw: toml::Table,
}

/// Read the config for the current environment.
///
/// `Ok(None)` when there is no `config.toml`, which is a normal state rather
/// than an error. `Err` only when a file exists and cannot be read or parsed.
pub fn load() -> Result<Option<CodexConfig>> {
    let home = crate::codex_home::resolve(&|key| std::env::var(key).ok());
    load_from_home(home)
}

/// [`load`], but against an explicit `CODEX_HOME`.
pub fn load_from_home(codex_home: impl AsRef<Path>) -> Result<Option<CodexConfig>> {
    let home = codex_home.as_ref();
    let path = home.join("config.toml");

    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(Error::Io {
                message: format!("failed to read {}: {e}", path.display()),
                source: e,
                working_dir: Some(home.to_path_buf()),
            });
        }
    };

    let raw: toml::Table = contents
        .parse::<toml::Table>()
        .map_err(|e| Error::ConfigParse {
            path: path.clone(),
            message: e.to_string(),
        })?;

    Ok(Some(CodexConfig {
        model: string_at(&raw, "model"),
        approval_policy: string_at(&raw, "approval_policy"),
        sandbox_mode: string_at(&raw, "sandbox_mode"),
        web_search: string_at(&raw, "web_search"),
        features: bool_table(&raw, "features"),
        project_trust: project_trust(&raw),
        profiles: profile_files(home),
        legacy_profiles: table_keys(&raw, "profiles"),
        raw,
        path,
    }))
}

fn string_at(table: &toml::Table, key: &str) -> Option<String> {
    table.get(key)?.as_str().map(str::to_string)
}

fn bool_table(table: &toml::Table, key: &str) -> BTreeMap<String, bool> {
    table
        .get(key)
        .and_then(toml::Value::as_table)
        .map(|features| {
            features
                .iter()
                .filter_map(|(name, value)| Some((name.clone(), value.as_bool()?)))
                .collect()
        })
        .unwrap_or_default()
}

fn table_keys(table: &toml::Table, key: &str) -> Vec<String> {
    table
        .get(key)
        .and_then(toml::Value::as_table)
        .map(|inner| inner.keys().cloned().collect())
        .unwrap_or_default()
}

/// `[projects."<path>"] trust_level = "..."` flattened to path and level.
fn project_trust(table: &toml::Table) -> BTreeMap<String, String> {
    table
        .get("projects")
        .and_then(toml::Value::as_table)
        .map(|projects| {
            projects
                .iter()
                .filter_map(|(path, value)| {
                    let level = value.as_table()?.get("trust_level")?.as_str()?;
                    Some((path.clone(), level.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Profile names, from `<name>.config.toml` beside the base config.
///
/// An unreadable directory yields no profiles rather than an error: a missing
/// profile list should not fail a config read.
fn profile_files(home: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(home) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            // `config.toml` itself is the base, not a profile.
            let stem = name.strip_suffix(".config.toml")?;
            (!stem.is_empty()).then(|| stem.to_string())
        })
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "codex-wrapper-config-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(home: &Path, name: &str, contents: &str) {
        std::fs::write(home.join(name), contents).unwrap();
    }

    #[test]
    fn a_missing_config_is_not_an_error() {
        let home = temp_home("missing");
        assert_eq!(load_from_home(&home).unwrap(), None);
    }

    /// The key names and layout come from a real `~/.codex/config.toml`.
    #[test]
    fn reads_the_typed_keys() {
        let home = temp_home("typed");
        write(
            &home,
            "config.toml",
            r#"
model = "gpt-5.6-sol"
model_reasoning_effort = "high"
approval_policy = "on-request"
sandbox_mode = "workspace-write"
web_search = "live"

[features]
web-search = true
disabled-thing = false

[projects."/Users/someone/a-repo"]
trust_level = "trusted"
"#,
        );

        let config = load_from_home(&home).unwrap().unwrap();
        assert_eq!(config.model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(config.approval_policy.as_deref(), Some("on-request"));
        assert_eq!(config.sandbox_mode.as_deref(), Some("workspace-write"));
        assert_eq!(config.web_search.as_deref(), Some("live"));
        assert_eq!(config.features.get("web-search"), Some(&true));
        assert_eq!(config.features.get("disabled-thing"), Some(&false));
        assert_eq!(
            config
                .project_trust
                .get("/Users/someone/a-repo")
                .map(String::as_str),
            Some("trusted")
        );
    }

    /// A key this crate does not model must stay reachable, or the reader
    /// hides configuration from the host it is reporting for.
    #[test]
    fn untyped_keys_stay_in_raw() {
        let home = temp_home("raw");
        write(
            &home,
            "config.toml",
            "model = \"m\"\npersonality = \"terse\"\nservice_tier = \"priority\"\n",
        );

        let config = load_from_home(&home).unwrap().unwrap();
        assert_eq!(
            config.raw.get("personality").and_then(toml::Value::as_str),
            Some("terse")
        );
        // Typed keys are in raw too, so `raw` is the whole file.
        assert!(config.raw.contains_key("model"));
    }

    /// Profiles are `<name>.config.toml` files, not a table. Verified against
    /// 0.145.0, whose `--profile` help says it layers that file.
    #[test]
    fn profiles_come_from_the_files_beside_the_config() {
        let home = temp_home("profiles");
        write(&home, "config.toml", "model = \"base\"\n");
        write(&home, "work.config.toml", "model = \"work-model\"\n");
        write(
            &home,
            "personal.config.toml",
            "model = \"personal-model\"\n",
        );
        // Not a profile: no `.config.toml` suffix.
        write(&home, "notes.toml", "x = 1\n");

        let config = load_from_home(&home).unwrap().unwrap();
        assert_eq!(config.profiles, vec!["personal", "work"]);
        assert!(config.legacy_profiles.is_empty());
    }

    /// The CLI refuses to write these now, so an old config still carrying
    /// them should be visible rather than silently dropped.
    #[test]
    fn a_legacy_profiles_table_is_reported_separately() {
        let home = temp_home("legacy");
        write(
            &home,
            "config.toml",
            "[profiles.old]\nmodel = \"legacy-model\"\n",
        );

        let config = load_from_home(&home).unwrap().unwrap();
        assert_eq!(config.legacy_profiles, vec!["old"]);
        assert!(
            config.profiles.is_empty(),
            "a legacy table is not a usable profile"
        );
    }

    #[test]
    fn a_malformed_config_is_an_error_not_a_silent_default() {
        let home = temp_home("malformed");
        write(&home, "config.toml", "this is not = = toml");

        let err = load_from_home(&home).unwrap_err();
        assert!(
            matches!(err, Error::ConfigParse { .. }),
            "expected a parse error, got: {err:?}"
        );
        // It never ran a command, so it must not look like a command failure.
        assert_eq!(err.failure_kind(), None);
        assert_eq!(err.exit_code(), None);
    }
}
