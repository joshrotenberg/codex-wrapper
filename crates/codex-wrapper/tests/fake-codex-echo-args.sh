#!/usr/bin/env bash
# Fake codex that prints the arguments it was given, one per line.
#
# Used to check that the argv preview matches what a spawn actually receives.
# Comparing against this rather than against the assembly function is the
# point: a test that calls the same function the preview calls would pass even
# if the spawn path stopped using it.
for arg in "$@"; do
  echo "$arg"
done
