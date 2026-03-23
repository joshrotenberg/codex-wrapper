/// Start Codex as an MCP server (stdio).
///
/// Wraps `codex mcp-server`.
use crate::Codex;
use crate::command::CodexCommand;
use crate::error::Result;
use crate::exec::{self, CommandOutput};

/// Start Codex as an MCP server over stdio.
#[derive(Debug, Clone, Default)]
pub struct McpServerCommand {
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
}

impl McpServerCommand {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn config(mut self, key_value: impl Into<String>) -> Self {
        self.config_overrides.push(key_value.into());
        self
    }

    #[must_use]
    pub fn enable(mut self, feature: impl Into<String>) -> Self {
        self.enabled_features.push(feature.into());
        self
    }

    #[must_use]
    pub fn disable(mut self, feature: impl Into<String>) -> Self {
        self.disabled_features.push(feature.into());
        self
    }
}

impl CodexCommand for McpServerCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["mcp-server".into()];
        for v in &self.config_overrides {
            args.push("-c".into());
            args.push(v.clone());
        }
        for v in &self.enabled_features {
            args.push("--enable".into());
            args.push(v.clone());
        }
        for v in &self.disabled_features {
            args.push("--disable".into());
            args.push(v.clone());
        }
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_server_args() {
        assert_eq!(McpServerCommand::new().args(), vec!["mcp-server"]);
    }

    #[test]
    fn mcp_server_with_config_args() {
        let args = McpServerCommand::new()
            .config("model=\"gpt-5\"")
            .enable("web-search")
            .args();
        assert_eq!(
            args,
            vec![
                "mcp-server",
                "-c",
                "model=\"gpt-5\"",
                "--enable",
                "web-search"
            ]
        );
    }
}
