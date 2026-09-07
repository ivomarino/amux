#!/usr/bin/env bash
# Replace a file's CONTENT without reusing its inode.
#
#   scripts/atomic-replace.sh <new-content-file> <destination>
#
# WHY THIS EXISTS. `cp` and a plain shell redirect open the destination with
# O_TRUNC and keep the SAME inode. bash reads a script lazily, by byte OFFSET,
# so a shell already executing that file resumes at its old position inside the
# NEW bytes and runs a seam that exists in no version of the file. The symptom is
# `syntax error near unexpected token` on a line that is perfectly valid on disk,
# and `bash -n` passing seconds later is the SIGNATURE of this bug, not an
# exoneration — which is exactly why it gets written off as an unexplained flake.
# It killed four pushes across two lanes in one night, all on scripts every
# lane's push executes.
#
# rename(2) is a new inode plus an atomic directory-entry swap, so a running
# process finishes on the bytes it started with.
#
# The fleet CLAUDE.md has prescribed this path since before it existed; AF-533.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <new-content-file> <destination>" >&2
  exit 2
fi
src="$1"; dest="$2"

[[ -f "$src" ]] || { echo "atomic-replace: no such source file: $src" >&2; exit 1; }
[[ -e "$dest" ]] || { echo "atomic-replace: no such destination: $dest" >&2; exit 1; }
[[ -f "$dest" ]] || { echo "atomic-replace: destination is not a regular file: $dest" >&2; exit 1; }

# The temp file MUST live in the destination's own directory: rename(2) is only
# atomic within one filesystem, and /tmp is frequently a different one. A
# cross-device rename fails with EXDEV, and the obvious "fix" for that is a copy,
# which is the bug this script exists to avoid.
dest_dir="$(cd "$(dirname "$dest")" && pwd)"
tmp="$(mktemp "$dest_dir/.$(basename "$dest").XXXXXX")"
trap 'rm -f "$tmp"' EXIT INT TERM

cat "$src" > "$tmp"
# Carry the destination's mode across, or an executable becomes unrunnable and
# the next reader sees "permission denied" from a script that was fine.
chmod --reference="$dest" "$tmp" 2>/dev/null || chmod "$(stat -f '%Lp' "$dest")" "$tmp"

mv -f "$tmp" "$dest"      # mv within one filesystem is rename(2)
trap - EXIT INT TERM

echo "atomic-replace: $dest replaced via rename(2) (new inode; any running copy keeps the old one)"
