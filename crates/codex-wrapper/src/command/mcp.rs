use crate::Codex;
use crate::command::CodexCommand;
#[cfg(feature = "json")]
use crate::error::Error;
use crate::error::Result;
use crate::exec::{self, CommandOutput};

#[derive(Debug, Clone, Default)]
pub struct McpListCommand {
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    json: bool,
}

impl McpListCommand {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn json(mut self) -> Self {
        self.json = true;
        self
    }

    #[cfg(feature = "json")]
    pub async fn execute_json(&self, codex: &Codex) -> Result<serde_json::Value> {
        let mut args = self.args();
        if !self.json {
            args.push("--json".into());
        }

        let output = exec::run_codex(codex, args).await?;
        serde_json::from_str(&output.stdout).map_err(|source| Error::Json {
            message: "failed to parse MCP list output".into(),
            source,
        })
    }
}

impl CodexCommand for McpListCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = base_args(
            "list",
            &self.config_overrides,
            &self.enabled_features,
            &self.disabled_features,
        );
        if self.json {
            args.push("--json".into());
        }
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

#[derive(Debug, Clone, Default)]
pub struct McpGetCommand {
    name: String,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    json: bool,
}

impl McpGetCommand {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    #[must_use]
    pub fn json(mut self) -> Self {
        self.json = true;
        self
    }

    #[cfg(feature = "json")]
    pub async fn execute_json(&self, codex: &Codex) -> Result<serde_json::Value> {
        let mut args = self.args();
        if !self.json {
            args.push("--json".into());
        }
        let output = exec::run_codex(codex, args).await?;
        serde_json::from_str(&output.stdout).map_err(|source| Error::Json {
            message: "failed to parse MCP server output".into(),
            source,
        })
    }
}

impl CodexCommand for McpGetCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = base_args(
            "get",
            &self.config_overrides,
            &self.enabled_features,
            &self.disabled_features,
        );
        if self.json {
            args.push("--json".into());
        }
        args.push(self.name.clone());
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

#[derive(Debug, Clone)]
enum McpAddTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: Vec<String>,
    },
    Http {
        url: String,
        bearer_token_env_var: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct McpAddCommand {
    name: String,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    transport: McpAddTransport,
}

impl McpAddCommand {
    #[must_use]
    pub fn stdio(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            transport: McpAddTransport::Stdio {
                command: command.into(),
                args: Vec::new(),
                env: Vec::new(),
            },
        }
    }

    #[must_use]
    pub fn http(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            transport: McpAddTransport::Http {
                url: url.into(),
                bearer_token_env_var: None,
            },
        }
    }

    #[must_use]
    pub fn arg(mut self, value: impl Into<String>) -> Self {
        if let McpAddTransport::Stdio { args, .. } = &mut self.transport {
            args.push(value.into());
        }
        self
    }

    #[must_use]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        if let McpAddTransport::Stdio { env, .. } = &mut self.transport {
            env.push(format!("{}={}", key.into(), value.into()));
        }
        self
    }

    #[must_use]
    pub fn bearer_token_env_var(mut self, env_var: impl Into<String>) -> Self {
        if let McpAddTransport::Http {
            bearer_token_env_var,
            ..
        } = &mut self.transport
        {
            *bearer_token_env_var = Some(env_var.into());
        }
        self
    }
}

impl CodexCommand for McpAddCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = base_args(
            "add",
            &self.config_overrides,
            &self.enabled_features,
            &self.disabled_features,
        );
        args.push(self.name.clone());
        match &self.transport {
            McpAddTransport::Stdio {
                command,
                args: command_args,
                env,
            } => {
                for entry in env {
                    args.push("--env".into());
                    args.push(entry.clone());
                }
                args.push("--".into());
                args.push(command.clone());
                args.extend(command_args.clone());
            }
            McpAddTransport::Http {
                url,
                bearer_token_env_var,
            } => {
                args.push("--url".into());
                args.push(url.clone());
                if let Some(env_var) = bearer_token_env_var {
                    args.push("--bearer-token-env-var".into());
                    args.push(env_var.clone());
                }
            }
        }
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

#[derive(Debug, Clone)]
pub struct McpRemoveCommand {
    name: String,
}

impl McpRemoveCommand {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl CodexCommand for McpRemoveCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        vec!["mcp".into(), "remove".into(), self.name.clone()]
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

#[derive(Debug, Clone)]
pub struct McpLoginCommand {
    name: String,
    scopes: Option<String>,
}

impl McpLoginCommand {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            scopes: None,
        }
    }

    #[must_use]
    pub fn scopes(mut self, scopes: impl Into<String>) -> Self {
        self.scopes = Some(scopes.into());
        self
    }
}

impl CodexCommand for McpLoginCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["mcp".into(), "login".into()];
        if let Some(scopes) = &self.scopes {
            args.push("--scopes".into());
            args.push(scopes.clone());
        }
        args.push(self.name.clone());
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

#[derive(Debug, Clone)]
pub struct McpLogoutCommand {
    name: String,
}

impl McpLogoutCommand {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl CodexCommand for McpLogoutCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        vec!["mcp".into(), "logout".into(), self.name.clone()]
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

fn base_args(
    subcommand: &str,
    configs: &[String],
    enabled: &[String],
    disabled: &[String],
) -> Vec<String> {
    let mut args = vec!["mcp".into(), subcommand.into()];
    for value in configs {
        args.push("-c".into());
        args.push(value.clone());
    }
    for value in enabled {
        args.push("--enable".into());
        args.push(value.clone());
    }
    for value in disabled {
        args.push("--disable".into());
        args.push(value.clone());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_list_args() {
        assert_eq!(
            McpListCommand::new().json().args(),
            vec!["mcp", "list", "--json"]
        );
    }

    #[test]
    fn mcp_stdio_add_args() {
        let args = McpAddCommand::stdio("server", "uvx")
            .arg("my-server")
            .env("API_KEY", "secret")
            .args();
        assert_eq!(
            args,
            vec![
                "mcp",
                "add",
                "server",
                "--env",
                "API_KEY=secret",
                "--",
                "uvx",
                "my-server",
            ]
        );
    }

    #[test]
    fn mcp_http_add_args() {
        let args = McpAddCommand::http("server", "https://example.com/mcp")
            .bearer_token_env_var("TOKEN")
            .args();
        assert_eq!(
            args,
            vec![
                "mcp",
                "add",
                "server",
                "--url",
                "https://example.com/mcp",
                "--bearer-token-env-var",
                "TOKEN",
            ]
        );
    }
}
