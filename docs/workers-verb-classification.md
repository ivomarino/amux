# `/api/workers` verb classification (AF-203, epic AF-201)

Input: the 42-verb inventory an external contributor posted on
[#134](https://github.com/mixpeek/amux/issues/134), plus the dispatch tables in
`crates/amux-server/src/api/session_verbs.rs`. Enumerated from source on
2026-08-24: **26 GET verbs, 23 POST verbs, 45 distinct**.

Ethan's direction call, 2026-08-24: complete `/api/workers`. The legacy per-name
dispatcher is a compatibility surface, not the supported path.

## Why this document exists

The inventory's value is that the tail is **not uniform**. Promoting all 45
one-for-one would move the mess into the supported API and freeze today's naming
into it. So the disposition is decided once, here, rather than per-verb at
promotion time.

Every verb gets exactly one verdict. "LIFECYCLE, retire" is a verdict.

## The current surface

`GET /api/debug/routes` on the running server:

```
/api/workers                    GET, POST
/api/workers/{id}               GET, PATCH, DELETE
/api/workers/{id}/start         POST
/api/workers/{id}/stop          POST
/api/workers/{id}/peek          GET
/api/workers/{id}/dead-letters  GET
/api/workers/{name}/{*verb}     *          <- the other ~40
```

`/api/sessions/*` alias-rewrites onto `/api/workers/*` before routing
(`aliases::alias_layer`, `api/mod.rs:417`), so both spellings reach the same code.

## RESOURCE — operations on a worker as a resource. Promote.

| verb | method | note |
|---|---|---|
| `send` | POST | **Do this first.** The gap #134 names in its title. `peek` is already promoted and its counterpart is not, so a rust-managed worker can be observed and not driven. AF-202. |
| `report` | POST | The harness reporting its own state. This is D1's exit condition in `ethos.md` — the durable inverse of terminal scraping. It should be a first-class route, not a catch-all entry. |
| `steer` | GET | The steer queue is how board state reaches a lane at its turn boundary (the 2026-08-03 decision against a global bus). Load-bearing. |
| `keys` | POST | Raw keystrokes. **Not** a duplicate of `send`: `send` delivers a prompt at a turn boundary, `keys` writes to the terminal. Both are needed and the names should say which is which. |
| `resize` | POST | Terminal geometry. |
| `wake` | POST | |
| `clear` | POST | |
| `reset` | POST | |
| `apply-template` | POST | |

## DUPLICATE — another route already expresses this. Retire the verb.

| verb | method | survivor |
|---|---|---|
| `info` | GET | `GET /api/workers/{id}` |
| `meta` | GET | `GET /api/workers/{id}` |
| `rename` | POST | `PATCH /api/workers/{id}` |
| `archive` | POST | `PATCH /api/workers/{id}` |
| `delete` | POST | `DELETE /api/workers/{id}` |
| `done` | POST | `PATCH /api/workers/{id}` (a status write) |
| `clone` | POST | `duplicate` — pick ONE. They are near-identical and having both is how callers end up split across two spellings of the same act. |
| `simple` | GET | `GET /api/workers/{id}` with field selection, not a second endpoint whose only difference is how much it returns. |

Retiring here means the alias keeps answering until callers move; it does not mean
breaking them on the day the canonical route lands.

## SUB-RESOURCE — real capability, wrong shape. Group, do not promote flat.

Eight git verbs are operations on the worker's **checkout**, not on the worker:

`git`, `git-push`, `commits`, `commit-detail`, `commit-guard`, `diff`, `dirty`,
`tracked-files`

Promoting these flat would put eight sibling routes on a worker for a thing that
is one sub-resource. They belong under `/api/workers/{id}/git/...`. This is the
group where "awkward composition is a UX defect *in* the primitives" applies most
directly — fixing the shape here is worth more than routing around it.

## OBSERVABILITY READS — decide whether the worker is the right owner.

`log`, `transcript`, `transcripts`, `last-message`, `tasks`, `subagents`,
`stats`, `status-explain`, `search`

These predate the workers table and several are cross-cutting queries that happen
to be scoped to a worker. Before promoting any of them, check whether the question
is "what did this worker do" (worker sub-resource) or "what happened, filtered by
worker" (an observability query with a worker filter). The second belongs with the
request log and `/api/logs/*`, and promoting it onto the worker would be the
"ninth thing that re-expresses the primitives" that `CLAUDE.md` warns about.

**Not decided here.** This is the one group that needs a second pass with the
actual callers in hand, and saying so is a verdict rather than a deferral: no verb
in it should be promoted until that pass runs.

## CONFIG READS — fold into the scope API.

`memory`, `memory-inherited`, `memory-explain`, `env-explain`, `instructions`

These read per-scope config, which is what the uniform scope read/write endpoint
already exists for (`api/mod.rs`, AMUX-2608). Route them there rather than giving
the worker five bespoke config verbs.

## `manual`, `commit-report`

Unclassified pending a caller check. Named explicitly rather than omitted, because
an unlisted verb is indistinguishable from one nobody found.

## Acceptance for the epic

`AF-204`: the catch-all is deleted when every verb above has landed on its verdict.
The check that can fail is `GET /api/health/invariants` →
`route.callers_have_routes`, which enumerates SPA and CLI call sites against the
mounted table and names each miss. The route table alone cannot tell you a caller
was orphaned; that invariant can.
