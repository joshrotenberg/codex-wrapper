#!/usr/bin/env bash
# Fake codex that reproduces a real failure signature on stderr and exits 1.
#
# Each message is transcribed from a captured `codex-cli` 0.145.0 run, with
# the varying parts (request ids, cf-ray headers, paths) kept so the matching
# is exercised against the shape the CLI really emits, not a tidied version.
#
# Selected by CODEX_WRAPPER_TEST_FAILURE. Every one of these exits 1: the exit
# code carries no information, which is why classification reads stderr.
case "$CODEX_WRAPPER_TEST_FAILURE" in
  auth)
    echo "ERROR: Reconnecting... 5/5" >&2
    echo "ERROR: unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: https://api.openai.com/v1/responses, cf-ray: a272310168bcba62-SJC, request id: req_ef11f9069ab546a5ab970d005756b2bc" >&2
    ;;
  not-trusted)
    echo "Not inside a trusted directory and --skip-git-repo-check was not specified." >&2
    ;;
  config)
    echo "Error loading config.toml: unknown configuration field \`bogus\` in -c/--config override" >&2
    ;;
  session)
    echo "Error: thread/resume: thread/resume failed: no rollout found for thread id 00000000-0000-0000-0000-000000000000 (code -32600)" >&2
    ;;
  *)
    echo "some failure nobody has classified" >&2
    ;;
esac
exit 1
