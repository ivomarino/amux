#!/usr/bin/env python3
"""Move a VALIDATED frustrations.md entry into frustrations-archive.md.

Ethan, 2026-08-24: "its only verified when the originating session validates and
agrees its complete. once its complete delete from this md."

Deleting is the instruction; this is where the bytes go, and it exists because
`.claude/rules/frustrations.md` records what happens without it. A set-difference
over one file cannot see a MOVE and reports it as a deletion every time --
creative-dna measured 15 of 15 "lost" entries as archive moves, with the
restore/remove cycle run three times before anyone noticed. So the archive is not
sentiment: it is the thing that makes "was this lost or was it finished?"
answerable by a grep instead of by reading git history.

Every archived entry carries a VALIDATED line naming WHO signed it off and when.
The protocol's whole point is that the originating session is the only party who
can say an entry is done (AC-227: an entry marked fixed by somebody who was not
its author, over a card that had only half shipped), so an archive move with no
name on it would launder exactly the thing the protocol forbids.

Usage:
    scripts/frustrations-archive.py <line> <validated-by> <evidence...>
    scripts/frustrations-archive.py --list
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LEDGER = ROOT / "frustrations.md"
ARCHIVE = ROOT / "frustrations-archive.md"

ARCHIVE_HEADER = """# amux frustrations: archive

Entries retired from [`frustrations.md`](frustrations.md). An entry lands here only
when the session that ORIGINATED it said the friction is gone; the `VALIDATED:` line
names who said so and on what evidence.

This file exists so that "was this entry lost, or was it finished?" is a grep rather
than an archaeology exercise. A set-difference over the ledger alone cannot see a
MOVE and reports it as a deletion every time. Before restoring anything that looks
missing from `frustrations.md`, grep here first: present means it was retired on
purpose, and re-appending it manufactures a duplicate.

Nothing here is live. `frustrations.md` is the live file and the invariants
`frustrations.ledger_agrees_with_board` / `frustrations.cards_are_reachable` read
only that one.

---
"""


def parse(md):
    """Entry spans, keyed by the 1-based line of the `## ` heading.

    Same rule the Rust parser uses (crates/amux-server/src/invariants/checks.rs):
    entries start at a COLUMN-0 `## ` after the `---` that closes the header, so
    the header's deliberately-indented template cannot count itself.
    """
    lines = md.split("\n")
    start = next(i for i, l in enumerate(lines) if l.strip() == "---")
    heads = [i for i in range(start + 1, len(lines)) if lines[i].startswith("## ")]
    out = {}
    for n, i in enumerate(heads):
        end = heads[n + 1] if n + 1 < len(heads) else len(lines)
        out[i + 1] = (i, end, lines[i][3:].strip())
    return lines, out


def main():
    md = LEDGER.read_text()
    lines, spans = parse(md)
    if len(sys.argv) > 1 and sys.argv[1] == "--list":
        for ln, (_, _, title) in sorted(spans.items()):
            print(f"L{ln:<6} {title[:100]}")
        return 0
    if len(sys.argv) < 4:
        print(__doc__)
        return 2
    ln, who, evidence = int(sys.argv[1]), sys.argv[2], " ".join(sys.argv[3:])
    if ln not in spans:
        print(f"no entry starts at line {ln}. `--list` shows the heading lines.", file=sys.stderr)
        return 1
    i, end, title = spans[ln]
    body = lines[i:end]
    # Trim trailing blanks so the archive does not accumulate them.
    while body and not body[-1].strip():
        body.pop()
    stamped = [body[0]]
    stamped.append(f"VALIDATED: {who} | {evidence}")
    stamped.extend(body[1:])

    if not ARCHIVE.exists():
        ARCHIVE.write_text(ARCHIVE_HEADER)
    arch = ARCHIVE.read_text().rstrip("\n")
    ARCHIVE.write_text(arch + "\n\n" + "\n".join(stamped) + "\n")

    remaining = lines[:i] + lines[end:]
    LEDGER.write_text("\n".join(remaining))
    print(f"archived L{ln}: {title[:70]}")
    print(f"  validated by {who}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
