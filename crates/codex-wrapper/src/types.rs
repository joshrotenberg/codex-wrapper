use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    ReadOnly,
    #[default]
    WorkspaceWrite,
    DangerFullAccess,
}

impl SandboxMode {
    pub(crate) fn as_arg(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalPolicy {
    Untrusted,
    OnFailure,
    #[default]
    OnRequest,
    Never,
}

impl ApprovalPolicy {
    pub(crate) fn as_arg(self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::OnFailure => "on-failure",
            Self::OnRequest => "on-request",
            Self::Never => "never",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Color {
    Always,
    Never,
    #[default]
    Auto,
}

impl Color {
    pub(crate) fn as_arg(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Never => "never",
            Self::Auto => "auto",
        }
    }
}

#[cfg(feature = "json")]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JsonLineEvent {
    #[serde(rename = "type", default)]
    pub event_type: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl CliVersion {
    #[must_use]
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub fn parse_version_output(output: &str) -> Result<Self, VersionParseError> {
        output
            .split_whitespace()
            .find_map(|token| token.parse().ok())
            .ok_or_else(|| VersionParseError(output.trim().to_string()))
    }

    #[must_use]
    pub fn satisfies_minimum(&self, minimum: &CliVersion) -> bool {
        self >= minimum
    }
}

impl PartialOrd for CliVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CliVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
    }
}

impl fmt::Display for CliVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for CliVersion {
    type Err = VersionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err(VersionParseError(s.to_string()));
        }

        Ok(Self {
            major: parts[0]
                .parse()
                .map_err(|_| VersionParseError(s.to_string()))?,
            minor: parts[1]
                .parse()
                .map_err(|_| VersionParseError(s.to_string()))?,
            patch: parts[2]
                .parse()
                .map_err(|_| VersionParseError(s.to_string()))?,
        })
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("invalid version string: {0:?}")]
pub struct VersionParseError(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_version_output() {
        let version = CliVersion::parse_version_output("codex-cli 0.116.0").unwrap();
        assert_eq!(version, CliVersion::new(0, 116, 0));
    }

    #[test]
    fn parses_plain_version_output() {
        let version = CliVersion::parse_version_output("0.116.0").unwrap();
        assert_eq!(version, CliVersion::new(0, 116, 0));
    }
}
