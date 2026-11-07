#!/usr/bin/env bash
# Module-boundary check (constitution Principle III).
# Rule: module crates (catalog, enrollment, scorm, management) may depend on
# `contracts`, `shared`, and the standard Rust ecosystem — never on each other.
# `host` may depend on everything. A violation fails the build.
set -euo pipefail

cd "$(dirname "$0")/../.."

fail=0

# 1) Crate-level dependency graph (cargo metadata).
metafile="$(mktemp)"
cargo metadata --format-version 1 --no-deps > "$metafile" 2>/dev/null
python3 - "$metafile" <<'PY'
import json, sys
meta = json.load(open(sys.argv[1]))
modules = {"catalog", "enrollment", "scorm", "management"}
by_name = {p["name"]: p for p in meta["packages"]}
bad = []
for name in modules:
    pkg = by_name[name]
    for dep in pkg["dependencies"]:
        target = dep["name"]
        if target in modules and target != name:
            bad.append(f"{name} -> {target}")
if bad:
    print("BOUNDARY VIOLATION(S): " + ", ".join(bad))
    sys.exit(1)
print(f"boundary check: {len(modules)} module crates clean")
PY
rm -f "$metafile"

# 2) No module crate exposes cross-module re-exports or `include!` of another module.
for f in crates/catalog crates/enrollment crates/scorm crates/management; do
  if grep -rnE "(pub[[:space:]]+use|pub[[:space:]]+mod|include!)[[:space:]]*(catalog|enrollment|scorm|management)\b" "$f/src" 2>/dev/null; then
    echo "BOUNDARY VIOLATION in $f (cross-module symbol exposure)"
    fail=1
  fi
done

exit $fail
