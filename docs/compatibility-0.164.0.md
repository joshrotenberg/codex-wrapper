# Native CLI compatibility audit

Target: 0.164.0. Retained baseline: 0.145.0. Tracks #170.

This draft records the plan before compatibility changes. Results are pending.

- Select exact task-local native executables and record versions/source.
- Check emitted flags/configuration with authentication-free probes.
- Preserve the supported baseline; fix only demonstrated drift.
- Run required local format, feature/build, lint, test and documentation checks.
- Add or refresh pinned native CLI jobs and distinguish controlled process tests
  from native semantic verification.

Help/configuration contracts do not prove model behavior or all CLI semantics.
Fake executables test wrapper behavior, not native CLI compatibility.
