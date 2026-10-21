# Due JSONL parsing maintenance

This isolated branch starts at the exact revision consumed by frozen Due:
`1c0bbfa2a66024d094a8117ec83bca0d6b41232c` (codex-wrapper 0.4.4).
It is a defect-repair candidate, not a release or wrapper API extension.

Due issue 38 reproduces different outcomes for byte-identical native stdout
with buffering versus tracing. Buffered capture performs replacement decoding,
while streamed line decoding rejects invalid UTF-8; streamed JSONL also ignores
non-object lines that Due's buffered parser rejects.

Plan:
- Make the existing wrapper's decoding behavior consistent across its buffered
  and streamed paths, explicitly documenting the chosen policy.
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
