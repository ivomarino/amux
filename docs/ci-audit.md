# CI/CD audit — 2026-08-06

Measured over the last 100 workflow runs. The headline: **the failures are not
in the contribution gate.** `checks` — the workflow that actually gates commits
— is 25/26 green and runs in ~32s. Every meaningful failure is in the
cloud-deploy family, and they share one root cause.

## Run stats (last 100 runs)

| workflow | ok | fail | median secs |
|---|---:|---:|---:|
| Deploy to cloud.amux.io | 1 | **18** | 18 |
| Deploy amux.io | 14 | 3 | 40 |
| Daily backup — cloud container | 0 | **3** | 12 |
| Cloud image — build & push | 19 | 1 | 169 |
| **checks** (the gate) | **25** | 1 | **32** |
| Cloud recover | 0 | 1 | 136 |
| iOS — Nightly TestFlight | 3 | 0 | 300 |

## Root cause of the noise: one dead host, three workflows

`Deploy to cloud.amux.io`, `Daily backup`, and `Cloud recover` all die in
their **first SSH step** (median 12–18s — they fail before doing any work).
Cause is *not* a firewall or a wrong address, both of which were proposed and
disproved:

- DNS agrees with the workflows (`cloud.amux.io` → the same IP they hardcode).
- The host accepts TCP on 22 and 443 and then closes before the SSH banner —
  `ssh-keyscan` fails identically **from a laptop**, so it is not runner-specific.
- That is a userspace-not-servicing signature (AC-216/AC-229: global OOM, no
  container memory limits, gateway killed at 09:41).

So **18 of the 27 total failures are one sick host reported three times.** They
are not flaky CI; they are a truthful alarm about infrastructure, firing on
every push because `deploy-cloud.yml` triggers on `amux-server.py`.

**Recommendation (Ethan's call):** these three workflows should fail *once*
loudly, not on every push. Either gate the deploy on a reachability probe that
skips-with-notice when the host is down, or pause the workflow until AC-216 is
closed. Fixing the host closes all three.

## Coverage gaps found and closed

`checks` was green while three real defects shipped in one night. Two blind
spots, both now fixed:

1. **JS syntax parsed only the FIRST script block.** `re.search` instead of
   `re.findall` — 2,379 of 1,296,891 bytes. The 1.07 MB dashboard block, where
   every UI bug lives, was never parsed by CI while the step printed green.
   (The local pre-commit hook had been fixed months earlier; the CI copy had
   not — a fix applied to one copy of a duplicated check.)
2. **No check for the deleted-function class.** `node --check` proves a block
   *parses*, not that the names it calls *exist*. Three shipped bugs in one
   night were this exact shape (`switchView` → six deleted notes functions;
   `_gridRestoreLayout` and `wsLoadProfile` throwing on their first saved
   pane). Added `tests/check_client_refs.py`.
3. **Secrets were gated only by a local hook.** AC-239: four credentials
   committed since 2026-03-11 in a public repo, because the pre-commit hook was
   the only gate and its patterns matched none of them. The scan now runs in CI
   too, with the added patterns.

## Principles applied

- **Every check self-tests.** `check_client_refs.py` plants a ghost call that
  must be caught and a real name that must resolve; the CI secret scan plants a
  fake key before scanning. A check that cannot demonstrate it can fail is
  theatre — and the scanner that missed AC-239 was green the whole time.
- **Extraction is asserted, not assumed.** The refs check fails if the client
  extraction yields < 500 KB, so a regex that stops matching can never make the
  check pass vacuously.
- **Speed comes from scope, not from skipping.** `checks` stays ~32s because it
  is syntax + refs + secrets + pytest on one runner with no Docker build. The
  slow workflows (image build 169s, iOS 300s) are correctly out of the
  contribution path.
