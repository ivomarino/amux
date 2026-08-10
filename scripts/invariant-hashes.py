#!/usr/bin/env python3
# UTILITY, not the old Python server: rust-migration invariant-hash tooling, called by .github/workflows/rust.yml.
"""Invariant hash infrastructure (RR-0028j, Invariant 45).

Extracts every `### Invariant N:` section from docs/rust-rebuild-plan.md,
hashes its normative content, and maintains docs/invariant-hashes.json.

Modes:
  --write   regenerate the hash file (used when a clarification lands;
            the hash update must ride in the SAME commit as the doc edit)
  --check   CI mode: exit 1 when any invariant's content hash differs from
            the recorded one (a doc edit without a hash update = either an
            unreviewed weakening or a clarification missing its stamp —
            both block until a human or the author classifies the change),
            or when an invariant present in the hash file has VANISHED from
            the doc (deleting an invariant is the strongest weakening).

Content is normalized before hashing (whitespace collapsed) so reflowing a
paragraph does not read as a semantic change, while any word change does.
"""

import hashlib
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PLAN = REPO / "docs" / "rust-rebuild-plan.md"
HASHES = REPO / "docs" / "invariant-hashes.json"

SECTION_RE = re.compile(
    r"^### (Invariant \d+[a-z]?): (.+?)$", re.MULTILINE
)


def extract_invariants(text: str) -> dict:
    """Return {invariant_id: {"title":…, "hash":…}} for every invariant section."""
    matches = list(SECTION_RE.finditer(text))
    out = {}
    for i, m in enumerate(matches):
        start = m.end()
        # Section runs to the next "### " or "## " heading of any kind.
        nxt = re.search(r"^##+ ", text[start:], re.MULTILINE)
        end = start + nxt.start() if nxt else len(text)
        body = text[start:end]
        normalized = " ".join(body.split())
        digest = hashlib.sha256(normalized.encode()).hexdigest()[:16]
        inv_id = m.group(1)
        out[inv_id] = {"title": m.group(2).strip(), "hash": digest}
    return out


def main() -> int:
    mode = sys.argv[1] if len(sys.argv) > 1 else "--check"
    current = extract_invariants(PLAN.read_text())
    if not current:
        print("FAIL: no invariant sections found — extraction broke, which is "
              "itself a failure (a checker that finds nothing passes nothing)")
        return 1

    if mode == "--write":
        HASHES.write_text(json.dumps(current, indent=2, sort_keys=True) + "\n")
        print(f"wrote {len(current)} invariant hashes to {HASHES.relative_to(REPO)}")
        return 0

    if not HASHES.exists():
        print("FAIL: docs/invariant-hashes.json missing — run "
              "scripts/invariant-hashes.py --write in the same commit as the doc")
        return 1
    recorded = json.loads(HASHES.read_text())

    failures = []
    for inv_id, rec in recorded.items():
        cur = current.get(inv_id)
        if cur is None:
            failures.append(f"{inv_id} ({rec['title']}): VANISHED from the doc "
                            f"— deleting an invariant is a weakening; blocked")
        elif cur["hash"] != rec["hash"]:
            failures.append(
                f"{inv_id} ({cur['title']}): content changed "
                f"({rec['hash']} -> {cur['hash']}). If this is a clarification, "
                f"rerun --write in this commit; if it weakens the invariant, it "
                f"needs review (docs/proposed-amendments.md)")
    for inv_id, cur in current.items():
        if inv_id not in recorded:
            failures.append(f"{inv_id} ({cur['title']}): NEW invariant not in "
                            f"hash file — rerun --write in this commit")

    if failures:
        print(f"FAIL: {len(failures)} invariant hash violation(s):")
        for f in failures:
            print(f"  - {f}")
        return 1
    print(f"ok: {len(current)} invariants match recorded hashes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
