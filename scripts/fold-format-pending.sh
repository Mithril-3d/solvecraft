#!/usr/bin/env bash
# Release step: give the pending design-format changes their number for good.
#
# Moves every change in crates/doc/src/format/pending/ into crates/doc/src/format/v<N>/, where N
# is the number development builds have been writing for them (the last folded format + 1).
# Commit the result with the release's version bump. Nothing to do when nothing is pending.
# See crates/doc/src/format/pending/README.md.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
dir="$root/crates/doc/src/format"
last=$(sed -n 's/^const HAND_WRITTEN: u32 = \([0-9]*\);/\1/p' "$root/crates/doc/build.rs")
[ -n "$last" ] || { echo "HAND_WRITTEN not found in crates/doc/build.rs" >&2; exit 1; }
for d in "$dir"/v*/; do
  [ -d "$d" ] || continue
  n=${d%/}
  n=${n##*/v}
  if [ "$n" -gt "$last" ]; then last=$n; fi
done

shopt -s nullglob
pending=("$dir"/pending/*.rs)
if [ ${#pending[@]} -eq 0 ]; then
  echo "No pending format changes: this release writes format $last."
  exit 0
fi

next=$((last + 1))
mkdir -p "$dir/v$next"
git -C "$root" mv "${pending[@]}" "$dir/v$next/"
echo "Format $next is now fixed, with ${#pending[@]} change(s):"
for f in "${pending[@]}"; do echo "  ${f##*/}"; done
echo "Commit crates/doc/src/format/ with the version bump."
