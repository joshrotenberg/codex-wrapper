//! Check the installed CLI against the version range this wrapper is tested
//! against, before doing any real work.
//!
//! ```sh
//! cargo run --example health_check
//! ```

use codex_wrapper::{CliVersionStatus, Codex};

#[tokio::main]
async fn main() -> codex_wrapper::Result<()> {
    let codex = Codex::builder().build()?;

    let (min, max) = codex.tested_cli_version_range();
    println!("tested against: {min} ..= {max}");
    println!("installed:      {}", codex.cli_version().await?);

    // The soft check: report drift and decide for yourself.
    match codex.cli_version_status().await? {
        CliVersionStatus::Tested => println!("status:         tested"),
        CliVersionStatus::NewerUntested { found, tested_max } => {
            println!("status:         {found} is newer than the tested {tested_max}");
            println!("                should still work, semantics may have drifted");
        }
        CliVersionStatus::OlderThanMinimum { found, minimum } => {
            println!("status:         {found} is older than the tested {minimum}");
            println!("                some emitted flags are likely rejected");
        }
    }

    // The hard check: refuse to proceed outside the range. Use this at startup
    // in anything long-running, so drift surfaces once rather than as a
    // confusing failure later.
    match codex.ensure_tested_cli_version().await {
        Ok(version) => println!("\nok to proceed on {version}"),
        Err(e) => println!("\nrefusing to proceed: {e}"),
    }

    Ok(())
}
