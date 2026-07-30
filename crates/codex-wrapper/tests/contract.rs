//! Check every flag and config key this wrapper emits against the installed
//! `codex` CLI.
//!
//! The wrapper drifted silently from `codex-cli` 0.116 to 0.145 and nothing in
//! CI noticed. Three `exec` flags became invalid invocations, `--full-auto`
//! became a hard error on `fork` and `resume`, and the `sandbox` subcommand
//! changed shape. Each was found by hand, after the fact.
//!
//! This suite builds a maximal command from each builder, collects what it
//! emits, and checks it against the live binary. It answers one question:
//! **does the CLI still accept what we emit.**
//!
//! Run it with a `codex` binary in PATH:
//!
//! ```sh
//! cargo test --test contract -- --ignored
//! ```
//!
//! # How the two halves work
//!
//! **Named flags** are checked against `codex <subcommand> --help`. `clap`
//! indents each option two or six spaces, so option lines are unambiguous.
//! A flag we emit that no longer appears is drift. Note that this catches
//! *hidden* flags too: `--full-auto` still functions on `codex exec` but is
//! absent from help, which is precisely the state that precedes removal.
//!
//! **Config keys** (`-c key=value`) never appear in `--help`, so they are
//! probed separately. Passing `--strict-config` with a sentinel value makes
//! the CLI fail before it starts a session, and its error distinguishes the
//! two failure modes we care about:
//!
//! ```text
//! -c approval_policy="<sentinel>"
//!   -> unknown variant `<sentinel>`, expected one of `untrusted`, ...
//!      key exists, and the valid value set is enumerated for free
//!
//! -c removed_key="<sentinel>"
//!   -> unknown configuration field `removed_key` in -c/--config override
//!      key is gone
//! ```
//!
//! Because the sentinel is never valid, the probe always fails fast: no
//! session is started, no auth is needed, and no tokens are spent.
//!
//! # What this does not check
//!
//! Flag *semantics*, and flags the CLI offers that no builder wraps (see #65).
//! Value sets *are* checked for config keys, as a side effect of the probe,
//! but not for named flags.

use std::collections::BTreeSet;
use std::process::Command;

use codex_wrapper::{
    ApprovalPolicy, ApprovalPolicyConfig, CodexCommand, Color, ExecCommand, ExecResumeCommand,
    ForkCommand, ResumeCommand, ReviewCommand, SandboxMode, WebSearchMode,
};

/// Value used to force a config-key probe to fail. Must never be a valid
/// value for any key.
const SENTINEL: &str = "__codex_wrapper_contract_probe__";

/// Config keys this harness knows how to probe safely.
///
/// The probe works by passing a sentinel value the CLI must reject, which
/// makes it fail during config load rather than starting a session. That only
/// holds for keys with a closed value set. A free-form key like `model` would
/// happily accept the sentinel and go on to make a real API call, so probing
/// one is not safe.
///
/// A builder emitting a `-c` key that is not listed here fails the test rather
/// than being probed, so the unsafe case is caught before anything runs.
const ENUM_CONFIG_KEYS: &[&str] = &["approval_policy", "web_search", "sandbox_mode"];

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A builder's emitted arguments, split into the parts we check differently.
struct Emitted {
    /// Leading subcommand path, e.g. `["exec", "resume"]`.
    subcommand: Vec<String>,
    /// Named flags, e.g. `--sandbox`, `-c`.
    flags: BTreeSet<String>,
    /// `-c` payloads, as `(key, value)` with the value's quotes stripped.
    config_keys: Vec<(String, String)>,
}

/// Split a builder's argv into its subcommand path, flags, and config keys.
///
/// `subcommand_len` is how many leading tokens form the subcommand path;
/// builders always emit those first.
fn split(args: &[String], subcommand_len: usize) -> Emitted {
    let subcommand: Vec<String> = args[..subcommand_len].to_vec();
    let mut flags = BTreeSet::new();
    let mut config_keys = Vec::new();

    let mut rest = args[subcommand_len..].iter().peekable();
    while let Some(arg) = rest.next() {
        if !arg.starts_with('-') {
            continue;
        }
        flags.insert(arg.clone());
        if (arg == "-c" || arg == "--config")
            && let Some(payload) = rest.next()
            && let Some((key, value)) = payload.split_once('=')
        {
            config_keys.push((key.to_string(), value.trim_matches('"').to_string()));
        }
    }

    Emitted {
        subcommand,
        flags,
        config_keys,
    }
}

/// Run `codex <subcommand> <args...>` and return stdout and stderr merged.
///
/// Stdin is closed so the CLI never blocks waiting for a prompt.
fn run_codex(args: &[String]) -> String {
    let output = Command::new("codex")
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("codex binary must be in PATH; run with --ignored only when it is");
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The installed CLI's version string, for failure messages.
///
/// Drift reports are meaningless without it: the same failure means "we are
/// behind" on a new CLI and "we broke something" on the pinned one.
fn cli_version() -> String {
    run_codex(&["--version".to_string()]).trim().to_string()
}

/// Flags listed by `codex <subcommand> --help`.
///
/// `clap` renders each option on its own line indented two spaces (when it has
/// a short form) or six (when it does not). Restricting to those lines avoids
/// picking up flags mentioned in prose, which would mask a real removal.
fn help_flags(subcommand: &[String]) -> BTreeSet<String> {
    let mut args = subcommand.to_vec();
    args.push("--help".into());
    let help = run_codex(&args);

    let mut flags = BTreeSet::new();
    for line in help.lines() {
        let indent = line.len() - line.trim_start().len();
        if !(indent == 2 || indent == 6) {
            continue;
        }
        let trimmed = line.trim_start();
        if !trimmed.starts_with('-') {
            continue;
        }
        // Option lines look like `-c, --config <key=value>` or `--oss`.
        // Take tokens up to the first `<`, which begins the value placeholder.
        let head = trimmed.split('<').next().unwrap_or(trimmed);
        for token in head.split([',', ' ']) {
            let token = token.trim();
            if token.starts_with('-') && token.len() > 1 {
                flags.insert(token.to_string());
            }
        }
    }
    assert!(
        !flags.is_empty(),
        "no option lines parsed from `codex {} --help`; the help format changed \
         and this harness needs updating",
        subcommand.join(" ")
    );
    flags
}

/// Probe a config key, returning the values the CLI says it accepts.
///
/// Returns `Err` with the CLI's own message when the key no longer exists.
fn probe_config_key(subcommand: &[String], key: &str) -> Result<BTreeSet<String>, String> {
    // Config load runs before the CLI validates anything else, so the probe
    // needs no prompt and no other flags. Keeping it minimal means it works
    // identically on every subcommand.
    let mut args = subcommand.to_vec();
    args.push("--strict-config".into());
    args.push("-c".into());
    args.push(format!("{key}=\"{SENTINEL}\""));

    let output = run_codex(&args);

    if output.contains(&format!("unknown configuration field `{key}`")) {
        return Err(format!(
            "config key `{key}` no longer exists: {}",
            output.lines().next().unwrap_or("").trim()
        ));
    }

    let marker = format!("unknown variant `{SENTINEL}`, expected one of ");
    let Some(rest) = output.split(&marker).nth(1) else {
        return Err(format!(
            "probe of `{key}` produced no recognizable error; the CLI's config \
             diagnostics changed and this harness needs updating. Output:\n{output}"
        ));
    };

    Ok(rest
        .lines()
        .next()
        .unwrap_or("")
        .split(',')
        .map(|v| v.trim().trim_matches('`').to_string())
        .filter(|v| !v.is_empty())
        .collect())
}

/// Assert every flag and config key a builder emits is still accepted.
fn assert_contract(label: &str, args: Vec<String>, subcommand_len: usize) {
    let emitted = split(&args, subcommand_len);
    let accepted = help_flags(&emitted.subcommand);

    let mut drift = Vec::new();

    for flag in &emitted.flags {
        if !accepted.contains(flag) {
            drift.push(format!(
                "flag `{flag}` is not listed by `codex {} --help`",
                emitted.subcommand.join(" ")
            ));
        }
    }

    for (key, value) in &emitted.config_keys {
        if !ENUM_CONFIG_KEYS.contains(&key.as_str()) {
            drift.push(format!(
                "config key `{key}` has no probe policy; add it to ENUM_CONFIG_KEYS \
                 if its value set is closed, or teach the harness how to check it \
                 without starting a session"
            ));
            continue;
        }
        match probe_config_key(&emitted.subcommand, key) {
            Err(message) => drift.push(message),
            Ok(valid) => {
                if !valid.contains(value) {
                    drift.push(format!(
                        "config key `{key}` no longer accepts `{value}`; valid values are {}",
                        valid
                            .iter()
                            .map(|v| format!("`{v}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }
    }

    assert!(
        drift.is_empty(),
        "{label} has drifted from the installed CLI ({}):\n  - {}\n\nEmitted argv: {args:?}",
        cli_version(),
        drift.join("\n  - ")
    );
}

// ---------------------------------------------------------------------------
// Maximal builders
//
// Each constructs a command with every option set, so every flag and config
// key the builder can emit appears in its argv.
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn exec_contract() {
    let args = ExecCommand::new("probe")
        .approval_policy(ApprovalPolicyConfig::Granular)
        .search_mode(WebSearchMode::Live)
        .enable("feature")
        .disable("other")
        .image("/tmp/a.png")
        .model("o3")
        .oss()
        .local_provider("ollama")
        .sandbox(SandboxMode::WorkspaceWrite)
        .strict_config()
        .dangerously_bypass_hook_trust()
        .ignore_user_config()
        .ignore_rules()
        .profile("default")
        .dangerously_bypass_approvals_and_sandbox()
        .cd("/tmp")
        .skip_git_repo_check()
        .add_dir("/tmp/extra")
        .ephemeral()
        .output_schema("/tmp/schema.json")
        .color(Color::Never)
        .json()
        .output_last_message("/tmp/last.txt")
        .args();
    assert_contract("ExecCommand", args, 1);
}

/// `full_auto()` is exercised separately: it and `sandbox()` set the same
/// flag, and an explicit `sandbox()` call wins, so a maximal builder would
/// never emit what `full_auto()` produces.
#[test]
#[ignore]
fn exec_full_auto_contract() {
    let args = ExecCommand::new("probe").full_auto().args();
    assert_contract("ExecCommand::full_auto", args, 1);
}

#[test]
#[ignore]
fn exec_resume_contract() {
    let args = ExecResumeCommand::new()
        .last()
        .prompt("probe")
        .approval_policy(ApprovalPolicyConfig::OnFailure)
        .search_mode(WebSearchMode::Cached)
        .enable("feature")
        .disable("other")
        .image("/tmp/a.png")
        .model("o3")
        .strict_config()
        .dangerously_bypass_hook_trust()
        .ignore_user_config()
        .ignore_rules()
        .output_schema("/tmp/schema.json")
        .dangerously_bypass_approvals_and_sandbox()
        .skip_git_repo_check()
        .ephemeral()
        .json()
        .output_last_message("/tmp/last.txt")
        .args();
    assert_contract("ExecResumeCommand", args, 2);
}

#[test]
#[ignore]
fn exec_resume_full_auto_contract() {
    let args = ExecResumeCommand::new().last().full_auto().args();
    assert_contract("ExecResumeCommand::full_auto", args, 2);
}

#[test]
#[ignore]
fn review_contract() {
    let args = ReviewCommand::new()
        .prompt("probe")
        .approval_policy(ApprovalPolicyConfig::Never)
        .search_mode(WebSearchMode::Indexed)
        .enable("feature")
        .disable("other")
        .uncommitted()
        .model("o3")
        .title("probe")
        .strict_config()
        .dangerously_bypass_hook_trust()
        .ignore_user_config()
        .ignore_rules()
        .output_schema("/tmp/schema.json")
        .dangerously_bypass_approvals_and_sandbox()
        .skip_git_repo_check()
        .ephemeral()
        .json()
        .output_last_message("/tmp/last.txt")
        .args();
    assert_contract("ReviewCommand", args, 2);
}

#[test]
#[ignore]
fn review_full_auto_contract() {
    let args = ReviewCommand::new().uncommitted().full_auto().args();
    assert_contract("ReviewCommand::full_auto", args, 2);
}

#[test]
#[ignore]
fn fork_contract() {
    let args = ForkCommand::new()
        .last()
        .prompt("probe")
        .enable("feature")
        .disable("other")
        .image("/tmp/a.png")
        .model("o3")
        .oss()
        .local_provider("ollama")
        .profile("default")
        .sandbox(SandboxMode::WorkspaceWrite)
        .approval_policy(ApprovalPolicy::Never)
        .dangerously_bypass_approvals_and_sandbox()
        .dangerously_bypass_hook_trust()
        .strict_config()
        .no_alt_screen()
        .remote("ws://127.0.0.1:9000")
        .remote_auth_token_env("CODEX_TOKEN")
        .cd("/tmp")
        .search()
        .add_dir("/tmp/extra")
        .args();
    assert_contract("ForkCommand", args, 1);
}

#[test]
#[ignore]
fn fork_full_auto_contract() {
    let args = ForkCommand::new().last().full_auto().args();
    assert_contract("ForkCommand::full_auto", args, 1);
}

#[test]
#[ignore]
fn resume_contract() {
    let args = ResumeCommand::new()
        .last()
        .prompt("probe")
        .enable("feature")
        .disable("other")
        .image("/tmp/a.png")
        .model("o3")
        .oss()
        .local_provider("ollama")
        .profile("default")
        .sandbox(SandboxMode::WorkspaceWrite)
        .approval_policy(ApprovalPolicy::Never)
        .dangerously_bypass_approvals_and_sandbox()
        .dangerously_bypass_hook_trust()
        .strict_config()
        .no_alt_screen()
        .include_non_interactive()
        .remote("ws://127.0.0.1:9000")
        .remote_auth_token_env("CODEX_TOKEN")
        .cd("/tmp")
        .search()
        .add_dir("/tmp/extra")
        .args();
    assert_contract("ResumeCommand", args, 1);
}

#[test]
#[ignore]
fn resume_full_auto_contract() {
    let args = ResumeCommand::new().last().full_auto().args();
    assert_contract("ResumeCommand::full_auto", args, 1);
}

// ---------------------------------------------------------------------------
// Regression guards for the drift that has already bitten
// ---------------------------------------------------------------------------

/// The three flags #41 P1 removed from `codex exec`, plus `--full-auto`.
/// If any reappears in help, the CLI restored it and the wrapper's config-key
/// workarounds can be revisited.
#[test]
#[ignore]
fn removed_exec_flags_are_still_absent() {
    let accepted = help_flags(&["exec".to_string()]);
    for flag in ["--ask-for-approval", "--search", "--progress-cursor"] {
        assert!(
            !accepted.contains(flag),
            "`{flag}` is listed by `codex exec --help` again; the config-key \
             workaround added for #53 may no longer be needed"
        );
    }
}

/// `--full-auto` is rejected outright by `fork` and `resume` (#55). No builder
/// may emit it on any subcommand.
#[test]
#[ignore]
fn no_builder_emits_full_auto() {
    let commands: Vec<(&str, Vec<String>)> = vec![
        ("ExecCommand", ExecCommand::new("probe").full_auto().args()),
        (
            "ExecResumeCommand",
            ExecResumeCommand::new().last().full_auto().args(),
        ),
        (
            "ReviewCommand",
            ReviewCommand::new().uncommitted().full_auto().args(),
        ),
        ("ForkCommand", ForkCommand::new().last().full_auto().args()),
        (
            "ResumeCommand",
            ResumeCommand::new().last().full_auto().args(),
        ),
    ];
    for (label, args) in commands {
        assert!(
            !args.iter().any(|a| a == "--full-auto"),
            "{label} emits --full-auto: {args:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Self-tests
//
// A contract check that cannot fail is worse than none: it reads as coverage
// while proving nothing. These confirm each detection path fires.
// ---------------------------------------------------------------------------

/// A flag the CLI does not have must be reported.
#[test]
#[ignore]
fn harness_detects_a_removed_flag() {
    let args = vec![
        "exec".to_string(),
        "--codex-wrapper-not-a-real-flag".to_string(),
    ];
    let result = std::panic::catch_unwind(|| assert_contract("synthetic", args, 1));
    assert!(
        result.is_err(),
        "harness accepted a flag the CLI does not have"
    );
}

/// A config key the CLI no longer recognizes must be reported.
#[test]
#[ignore]
fn harness_detects_a_removed_config_key() {
    let error = probe_config_key(&["exec".to_string()], "codex_wrapper_not_a_real_key")
        .expect_err("harness accepted a config key the CLI does not have");
    assert!(
        error.contains("no longer exists"),
        "expected a removed-key diagnosis, got: {error}"
    );
}

/// A value outside a config key's set must be reported.
#[test]
#[ignore]
fn harness_detects_a_removed_config_value() {
    let valid = probe_config_key(&["exec".to_string()], "approval_policy")
        .expect("approval_policy should still exist");
    assert!(
        !valid.contains("not-a-real-policy"),
        "probe returned a value set that accepts anything: {valid:?}"
    );
    // Sanity-check the parse while we are here: these are the values the CLI
    // enumerated when this was written.
    assert!(
        valid.contains("never") && valid.contains("granular"),
        "value set parsed as {valid:?}, which does not look like approval_policy"
    );
}

/// The harness must refuse to probe a key whose value set is open, rather than
/// launching a real session against it.
#[test]
#[ignore]
fn harness_refuses_to_probe_free_form_config_keys() {
    assert!(
        !ENUM_CONFIG_KEYS.contains(&"model"),
        "`model` is free-form; probing it would start a real session"
    );
    let args = vec![
        "exec".to_string(),
        "-c".to_string(),
        "model=\"o3\"".to_string(),
    ];
    let result = std::panic::catch_unwind(|| assert_contract("synthetic", args, 1));
    assert!(
        result.is_err(),
        "harness probed a free-form config key instead of refusing"
    );
}
