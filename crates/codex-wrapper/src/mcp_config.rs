//! Build MCP server configuration for a single run, without touching the
//! user's persistent config.
//!
//! [`McpAddCommand`](crate::McpAddCommand) and friends mutate
//! `$CODEX_HOME/config.toml`. That is the wrong tool for a host running many
//! isolated invocations: a cancelled run leaves residue, and two overlapping
//! runs race each other.
//!
//! # Why overrides rather than a config file
//!
//! `claude-wrapper`'s equivalent writes a JSON file and passes it to
//! `--mcp-config`. Codex has no such flag. Checked against 0.145.0: the only
//! config-bearing options on `codex exec` are `-c/--config`, `--profile`,
//! which layers `$CODEX_HOME/<name>.config.toml`, and `--ignore-user-config`.
//! Nothing consumes a standalone server-config file.
//!
//! So the per-run mechanism here is `-c` overrides, which suits the purpose
//! better than a file would: nothing is written, nothing is left behind when a
//! run is cancelled, and concurrent runs cannot collide.
//!
//! For the cases that do want a file, [`McpConfigBuilder::to_toml`] and
//! [`McpConfigBuilder::write_profile`] produce a profile that `--profile`
//! layers.
//!
//! # Verified forms
//!
//! Each of these was accepted by `codex exec --strict-config`, which rejects
//! a malformed server outright (a table with no transport fails with
//! `invalid transport`):
//!
//! ```text
//! mcp_servers.<name>.command="npx"
//! mcp_servers.<name>.args=["-y","server"]
//! mcp_servers.<name>.env={API_KEY="x"}
//! mcp_servers.<name>.url="https://example.com/mcp"
//! mcp_servers.<name>.bearer_token_env_var="TOKEN"
//! mcp_servers.<name>.env_http_headers={X-Identity="IDENTITY_TOKEN"}
//! mcp_servers.<name>.required=true
//! ```
//!
//! # Example
//!
//! ```
//! use codex_wrapper::{ExecCommand, McpConfigBuilder};
//!
//! let mcp = McpConfigBuilder::new()
//!     .stdio_server("files", "npx")
//!     .http_server("docs", "https://example.com/mcp");
//!
//! let mut cmd = ExecCommand::new("summarize the docs");
//! for override_ in mcp.config_overrides() {
//!     cmd = cmd.config(override_);
//! }
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// How a server is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Transport {
    Stdio {
        command: String,
        args: Vec<String>,
    },
    Http {
        url: String,
        bearer_token_env_var: Option<String>,
        env_http_headers: BTreeMap<String, String>,
    },
}

/// One MCP server's configuration.
///
/// Mirrors what [`McpAddCommand`](crate::McpAddCommand) can register, so the
/// two describe the same thing whether it is persisted or scoped to a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    transport: Transport,
    env: BTreeMap<String, String>,
    required: bool,
}

impl McpServerConfig {
    /// A server launched as a subprocess.
    #[must_use]
    pub fn stdio(command: impl Into<String>) -> Self {
        Self {
            transport: Transport::Stdio {
                command: command.into(),
                args: Vec::new(),
            },
            env: BTreeMap::new(),
            required: false,
        }
    }

    /// A server reached over HTTP.
    #[must_use]
    pub fn http(url: impl Into<String>) -> Self {
        Self {
            transport: Transport::Http {
                url: url.into(),
                bearer_token_env_var: None,
                env_http_headers: BTreeMap::new(),
            },
            env: BTreeMap::new(),
            required: false,
        }
    }

    /// Append a launch argument. Ignored for an HTTP server.
    #[must_use]
    pub fn arg(mut self, value: impl Into<String>) -> Self {
        if let Transport::Stdio { args, .. } = &mut self.transport {
            args.push(value.into());
        }
        self
    }

    /// Set an environment variable for the subprocess.
    #[must_use]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Name the environment variable holding the bearer token.
    ///
    /// The token itself is never part of the configuration: only the name of
    /// the variable to read it from, matching `mcp add --bearer-token-env-var`.
    /// Ignored for a stdio server.
    #[must_use]
    pub fn bearer_token_env_var(mut self, env_var: impl Into<String>) -> Self {
        if let Transport::Http {
            bearer_token_env_var,
            ..
        } = &mut self.transport
        {
            *bearer_token_env_var = Some(env_var.into());
        }
        self
    }

    /// Source an HTTP request header from an environment variable.
    ///
    /// Both arguments are names: `header` is sent on each request and
    /// `env_var` is read by Codex for its value. The secret itself therefore
    /// stays out of argv and persistent configuration. Ignored for a stdio
    /// server.
    #[must_use]
    pub fn env_http_header(
        mut self,
        header: impl Into<String>,
        env_var: impl Into<String>,
    ) -> Self {
        if let Transport::Http {
            env_http_headers, ..
        } = &mut self.transport
        {
            env_http_headers.insert(header.into(), env_var.into());
        }
        self
    }

    /// Require this server to initialize successfully.
    ///
    /// Codex otherwise treats an unavailable MCP server as a warning and may
    /// continue without capabilities the caller expected to be present.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// `key = value` pairs for this server, without the `mcp_servers.<name>.`
    /// prefix.
    fn fields(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        match &self.transport {
            Transport::Stdio { command, args } => {
                out.push(("command".into(), toml_string(command)));
                if !args.is_empty() {
                    let items: Vec<String> = args.iter().map(|a| toml_string(a)).collect();
                    out.push(("args".into(), format!("[{}]", items.join(","))));
                }
            }
            Transport::Http {
                url,
                bearer_token_env_var,
                env_http_headers,
            } => {
                out.push(("url".into(), toml_string(url)));
                if let Some(var) = bearer_token_env_var {
                    out.push(("bearer_token_env_var".into(), toml_string(var)));
                }
                if !env_http_headers.is_empty() {
                    let pairs: Vec<String> = env_http_headers
                        .iter()
                        .map(|(header, var)| format!("{}={}", toml_key(header), toml_string(var)))
                        .collect();
                    out.push((
                        "env_http_headers".into(),
                        format!("{{{}}}", pairs.join(",")),
                    ));
                }
            }
        }
        if !self.env.is_empty() {
            let pairs: Vec<String> = self
                .env
                .iter()
                .map(|(k, v)| format!("{}={}", toml_key(k), toml_string(v)))
                .collect();
            out.push(("env".into(), format!("{{{}}}", pairs.join(","))));
        }
        if self.required {
            out.push(("required".into(), "true".into()));
        }
        out
    }
}

/// A set of MCP servers for one run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpConfigBuilder {
    servers: BTreeMap<String, McpServerConfig>,
}

impl McpConfigBuilder {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a server.
    #[must_use]
    pub fn server(mut self, name: impl Into<String>, config: McpServerConfig) -> Self {
        self.servers.insert(name.into(), config);
        self
    }

    /// Add a subprocess server. Shorthand for [`McpServerConfig::stdio`].
    #[must_use]
    pub fn stdio_server(self, name: impl Into<String>, command: impl Into<String>) -> Self {
        self.server(name, McpServerConfig::stdio(command))
    }

    /// Add an HTTP server. Shorthand for [`McpServerConfig::http`].
    #[must_use]
    pub fn http_server(self, name: impl Into<String>, url: impl Into<String>) -> Self {
        self.server(name, McpServerConfig::http(url))
    }

    /// `key=value` strings for
    /// [`ExecCommand::config`](crate::ExecCommand::config), or for the client
    /// builder's `config` when the whole client should carry them.
    ///
    /// Ordered, so the same set produces the same arguments.
    #[must_use]
    pub fn config_overrides(&self) -> Vec<String> {
        self.servers
            .iter()
            .flat_map(|(name, config)| {
                config.fields().into_iter().map(move |(key, value)| {
                    format!("mcp_servers.{}.{key}={value}", toml_key(name))
                })
            })
            .collect()
    }

    /// The same configuration as a TOML document.
    ///
    /// Suitable for a `$CODEX_HOME/<name>.config.toml` profile, which
    /// `--profile` layers over the base config.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        for (name, config) in &self.servers {
            out.push_str(&format!("[mcp_servers.{}]\n", toml_key(name)));
            for (key, value) in config.fields() {
                out.push_str(&format!("{key} = {value}\n"));
            }
            out.push('\n');
        }
        out
    }

    /// Write [`to_toml`](Self::to_toml) to `$CODEX_HOME/<profile>.config.toml`
    /// and return the path.
    ///
    /// Reachable afterwards as `--profile <profile>`. This writes into the
    /// user's codex home, so it is persistent: prefer
    /// [`config_overrides`](Self::config_overrides) for a single run.
    pub fn write_profile(&self, codex_home: impl AsRef<Path>, profile: &str) -> Result<PathBuf> {
        let path = codex_home.as_ref().join(format!("{profile}.config.toml"));
        std::fs::write(&path, self.to_toml()).map_err(|e| Error::Io {
            message: format!("failed to write {}: {e}", path.display()),
            source: e,
            working_dir: Some(codex_home.as_ref().to_path_buf()),
        })?;
        Ok(path)
    }

    /// Whether any server has been added.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }
}

/// A TOML basic string, quoted and escaped.
fn toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A TOML key, bare when it can be and quoted when it cannot.
///
/// Server names come from the caller, so a name with a dot would otherwise
/// silently become a nested table rather than a server called `a.b`.
fn toml_key(key: &str) -> String {
    let bare = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        key.to_string()
    } else {
        toml_string(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exactly the forms accepted by `codex exec --strict-config` on 0.145.0.
    #[test]
    fn overrides_match_the_verified_forms() {
        let mcp = McpConfigBuilder::new()
            .server(
                "files",
                McpServerConfig::stdio("npx").arg("-y").arg("server"),
            )
            .server(
                "docs",
                McpServerConfig::http("https://example.com/mcp")
                    .bearer_token_env_var("TOKEN")
                    .env_http_header("X-Identity", "IDENTITY_TOKEN")
                    .required(),
            );

        assert_eq!(
            mcp.config_overrides(),
            vec![
                r#"mcp_servers.docs.url="https://example.com/mcp""#,
                r#"mcp_servers.docs.bearer_token_env_var="TOKEN""#,
                r#"mcp_servers.docs.env_http_headers={X-Identity="IDENTITY_TOKEN"}"#,
                "mcp_servers.docs.required=true",
                r#"mcp_servers.files.command="npx""#,
                r#"mcp_servers.files.args=["-y","server"]"#,
            ]
        );
    }

    #[test]
    fn env_becomes_an_inline_table() {
        let mcp = McpConfigBuilder::new().server(
            "files",
            McpServerConfig::stdio("run").env("B", "2").env("A", "1"),
        );

        assert_eq!(
            mcp.config_overrides(),
            vec![
                r#"mcp_servers.files.command="run""#,
                r#"mcp_servers.files.env={A="1",B="2"}"#,
            ],
            "entries are ordered, so the same set produces the same arguments"
        );
    }

    /// A value carrying a quote or a backslash must not break out of the TOML
    /// string and turn into a different override than intended.
    #[test]
    fn values_are_escaped() {
        let mcp = McpConfigBuilder::new().server(
            "s",
            McpServerConfig::stdio(r#"say "hi""#)
                .arg("back\\slash")
                .arg("two\nlines"),
        );

        let overrides = mcp.config_overrides();
        assert_eq!(overrides[0], r#"mcp_servers.s.command="say \"hi\"""#);
        assert_eq!(
            overrides[1],
            r#"mcp_servers.s.args=["back\\slash","two\nlines"]"#
        );
    }

    /// A dotted name would otherwise become a nested table rather than a
    /// server whose name contains a dot.
    #[test]
    fn a_name_needing_quotes_gets_them() {
        let mcp = McpConfigBuilder::new().stdio_server("my.server", "run");
        assert_eq!(
            mcp.config_overrides(),
            vec![r#"mcp_servers."my.server".command="run""#]
        );
    }

    #[test]
    fn args_and_bearer_token_apply_only_where_they_belong() {
        // An HTTP server ignores launch args; a stdio server ignores the token.
        let http =
            McpConfigBuilder::new().server("h", McpServerConfig::http("https://x").arg("-y"));
        assert_eq!(
            http.config_overrides(),
            vec![r#"mcp_servers.h.url="https://x""#]
        );

        let stdio = McpConfigBuilder::new()
            .server("s", McpServerConfig::stdio("run").bearer_token_env_var("T"));
        assert_eq!(
            stdio.config_overrides(),
            vec![r#"mcp_servers.s.command="run""#]
        );

        let stdio = McpConfigBuilder::new().server(
            "s",
            McpServerConfig::stdio("run").env_http_header("X-Identity", "TOKEN"),
        );
        assert_eq!(
            stdio.config_overrides(),
            vec![r#"mcp_servers.s.command="run""#]
        );
    }

    #[test]
    fn env_backed_http_headers_are_ordered_and_escape_header_names() {
        let mcp = McpConfigBuilder::new().server(
            "api",
            McpServerConfig::http("https://example.com/mcp")
                .env_http_header("x.second", "SECOND_TOKEN")
                .env_http_header("x-first", "FIRST_TOKEN"),
        );

        assert_eq!(
            mcp.config_overrides(),
            vec![
                r#"mcp_servers.api.url="https://example.com/mcp""#,
                r#"mcp_servers.api.env_http_headers={x-first="FIRST_TOKEN","x.second"="SECOND_TOKEN"}"#,
            ]
        );
    }

    #[test]
    fn to_toml_produces_a_profile_document() {
        let mcp = McpConfigBuilder::new().server("files", McpServerConfig::stdio("npx").arg("-y"));

        assert_eq!(
            mcp.to_toml(),
            "[mcp_servers.files]\ncommand = \"npx\"\nargs = [\"-y\"]\n\n"
        );
    }

    #[test]
    fn write_profile_lands_where_profile_would_look() {
        let home = std::env::temp_dir().join(format!("codex-wrapper-mcp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();

        let path = McpConfigBuilder::new()
            .stdio_server("files", "npx")
            .write_profile(&home, "isolated")
            .unwrap();

        assert_eq!(path, home.join("isolated.config.toml"));
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("[mcp_servers.files]"), "{written}");

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_empty_builder_produces_nothing() {
        let mcp = McpConfigBuilder::new();
        assert!(mcp.is_empty());
        assert!(mcp.config_overrides().is_empty());
        assert_eq!(mcp.to_toml(), "");
    }
}
