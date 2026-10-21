# Due JSONL parsing maintenance

This isolated branch starts at the exact revision consumed by frozen Due:
`1c0bbfa2a66024d094a8117ec83bca0d6b41232c` (codex-wrapper 0.4.4).
It is a defect-repair candidate, not a release or wrapper API extension.

Due issue 38 reproduces different outcomes for byte-identical native stdout
with buffering versus tracing. Buffered capture performs replacement decoding,
while streamed line decoding rejects invalid UTF-8; streamed JSONL also ignores
non-object lines that Due's buffered parser rejects.

Plan:
- Match buffered replacement decoding only in exec stdout/stderr streaming.
  Preserve app-server's existing strict UTF-8 helper and behavior.
- Keep command construction, public API, features, retry configuration, process
  ownership, cancellation and timeout behavior unchanged.
- Add deterministic malformed-byte and framing regressions using known event
  shapes; deliberately malformed bytes remain synthetic fault injection.
- In a separate Due candidate, align its JSONL line policy and preserve available
  partial output without changing failures into success.
- Verify the wrapper's required feature matrix and Rust 1.90 compatibility, plus
  Due's paired fake-provider and failure-precedence matrix.
- Use Due for implementation and an independent other-provider review.
  The caller owns publication and artifact/effect verification.

No release, main merge, installed-CLI update, new runner feature, or inclusion
of unrelated newer wrapper changes is proposed.

Implementation evidence:
- Exec stdout/stderr now use a private replacement decoder after a complete
  bounded byte line is assembled. Raw bytes, including CRLF, count toward the
  existing per-stream quota before decoding. Fresh and resume share this path.
- App-server still calls `read_bounded_line` with its original strict decoding,
  error shape, framing and quota semantics. Its source is unchanged. Regressions
  reject invalid bytes on that helper while accepting an explicit valid U+FFFD.
- Exec regressions split UTF-8 across one-byte reads, compare replacement output
  with buffered capture, and check stdout/stderr raw-byte limits. Due's synthetic
  paired fixture also exercises malformed stdout and stderr without a model.
- Due's Git dependency pin and lock remain caller-owned. Local integration uses
  a command-scoped Cargo path patch and restores the original lock bytes.

Local validation used Rust 1.90 and offline synthetic fixtures. Formatting,
all-target/all-feature strict Clippy, the all-feature check and all 29 doctests
passed. All three new decoding regressions passed. The all-feature library suite
had 302 passes and 30 failures; no-default-features had 161 passes and 15 failures.
Every failure was an existing cleanup assertion denied access to `ps` by the
host, at unchanged `test_support.rs:368`. These suites need an unrestricted
caller rerun; their process-cleanup verification is not claimed as passing.

Isolated pre-repair checks failed on Due's diagnostic-line admission and Claude's
missing-terminal text retention. Repaired Due with the original pinned wrapper
still failed the invalid-UTF-8 stream case. No native model or real CLI contract
run was used. The caller still owns publication, the dependency revision update
and final Git-pinned locked integration checks.

Caller verification on 2026-10-21 reran the same wrapper source bytes in an
authorized host context with process inspection: all 332 all-feature library
tests, 176 no-default-feature tests and 29 doctests passed. Formatting, strict
all-target/all-feature Clippy and the Rust 1.90 all-feature check also passed.
The worker permission failures remain retained evidence; no native worker
grants were changed. Independent other-provider review is still pending.
