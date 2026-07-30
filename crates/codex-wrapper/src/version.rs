//! Version parsing and comparison utilities.
//!
//! Beyond parsing, this module declares the range of `codex-cli` versions this
//! wrapper is tested against, and classifies an installed binary against it
//! via [`CliVersionStatus`].
//!
//! The range is not an assertion of intent. Both bounds are exercised by the
//! contract check in `tests/contract.rs`, which runs against each of them in
//! CI and fails if any flag or config key the builders emit has stopped being
//! accepted. Bumping these constants means adding that version to the CI
//! matrix and fixing whatever drift the check reports.

pub use crate::types::{CliVersion, CliVersionStatus, VersionParseError};

/// Lowest `codex-cli` version this wrapper is tested against.
///
/// Older versions are not merely untested: 0.145.0 removed
/// `--ask-for-approval` and `--search` from the exec family in favor of
/// config keys (#53), so the arguments this wrapper emits are not accepted by
/// earlier releases.
pub const TESTED_CLI_VERSION_MIN: CliVersion = CliVersion {
    major: 0,
    minor: 145,
    patch: 0,
};

/// Highest `codex-cli` version this wrapper is tested against.
pub const TESTED_CLI_VERSION_MAX: CliVersion = CliVersion {
    major: 0,
    minor: 146,
    patch: 0,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tested_range_is_ordered() {
        assert!(
            TESTED_CLI_VERSION_MIN <= TESTED_CLI_VERSION_MAX,
            "tested range is inverted: {TESTED_CLI_VERSION_MIN}..={TESTED_CLI_VERSION_MAX}"
        );
    }

    /// The declared range is only meaningful if CI actually runs the contract
    /// check against both ends of it. Bumping one without the other would
    /// leave the crate claiming coverage it does not have, which is the exact
    /// failure mode this range exists to prevent.
    #[test]
    fn tested_range_matches_the_ci_contract_matrix() {
        let ci = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../.github/workflows/ci.yml"
        );
        let Ok(contents) = std::fs::read_to_string(ci) else {
            // Not in a repo checkout (vendored or packaged source). Nothing to
            // cross-check against.
            return;
        };

        let matrix = contents
            .lines()
            .find_map(|line| line.trim().strip_prefix("codex: ["))
            .expect("ci.yml should declare a `codex:` matrix for the contract job")
            .trim_end_matches(']')
            .split(',')
            .map(|v| v.trim().trim_matches('"').to_string())
            .collect::<Vec<_>>();

        for bound in [TESTED_CLI_VERSION_MIN, TESTED_CLI_VERSION_MAX] {
            assert!(
                matrix.contains(&bound.to_string()),
                "`{bound}` is a declared bound of the tested range but is not in \
                 ci.yml's contract matrix {matrix:?}; the range would claim \
                 coverage CI does not provide"
            );
        }
    }

    #[test]
    fn status_within_classifies_each_case() {
        let min = CliVersion::new(0, 145, 0);
        let max = CliVersion::new(0, 146, 0);

        assert_eq!(min.status_within(&min, &max), CliVersionStatus::Tested);
        assert_eq!(max.status_within(&min, &max), CliVersionStatus::Tested);
        assert_eq!(
            CliVersion::new(0, 145, 7).status_within(&min, &max),
            CliVersionStatus::Tested
        );

        assert_eq!(
            CliVersion::new(0, 144, 9).status_within(&min, &max),
            CliVersionStatus::OlderThanMinimum {
                found: CliVersion::new(0, 144, 9),
                minimum: min,
            }
        );
        assert_eq!(
            CliVersion::new(0, 147, 0).status_within(&min, &max),
            CliVersionStatus::NewerUntested {
                found: CliVersion::new(0, 147, 0),
                tested_max: max,
            }
        );
    }

    #[test]
    fn is_tested_is_true_only_for_tested() {
        let min = CliVersion::new(0, 145, 0);
        let max = CliVersion::new(0, 146, 0);
        assert!(min.status_within(&min, &max).is_tested());
        assert!(
            !CliVersion::new(0, 1, 0)
                .status_within(&min, &max)
                .is_tested()
        );
        assert!(
            !CliVersion::new(9, 0, 0)
                .status_within(&min, &max)
                .is_tested()
        );
    }
}
