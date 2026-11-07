#!/usr/bin/env bash
# Module-boundary check (constitution Principle III).
# Rule: module crates (catalog, enrollment, scorm, management) may depend on
# `contracts`, `shared`, and the standard Rust ecosystem — never on each other.
# `host` may depend on everything. A violation fails the build.
set -euo pipefail

cd "$(dirname "$0")/../.."

fail=0

# 1) Manifest-level dependency scan (deterministic, no cargo needed).
# NOTE: `cargo metadata --no-deps` does not enumerate direct dependencies —
# a manifest scan is the authoritative check for this rule (T022 finding).
for m in catalog enrollment scorm management; do
  for other in catalog enrollment scorm management; do
    if [ "$m" = "$other" ]; then continue; fi
    if grep -E "^${other}[[:space:]]*=" "crates/$m/Cargo.toml" 2>/dev/null; then
      echo "BOUNDARY VIOLATION: $m -> $other"
      fail=1
    fi
  done
done

# 2) No module crate exposes cross-module re-exports or `include!` of another module.
for f in crates/catalog crates/enrollment crates/scorm crates/management; do
  if grep -rnE "(pub[[:space:]]+use|pub[[:space:]]+mod|include!)[[:space:]]*(catalog|enrollment|scorm|management)\b" "$f/src" 2>/dev/null; then
    echo "BOUNDARY VIOLATION in $f (cross-module symbol exposure)"
    fail=1
  fi
done

if [ "$fail" -eq 0 ]; then
  echo "boundary check: 4 module crates clean"
fi

exit $fail
