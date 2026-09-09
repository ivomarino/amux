# Amux Business

A standalone Business frontend in the Amux repository, running locally at `http://127.0.0.1:3100/` and connected to the existing Amux server. Home, Work, Approvals, Automations, Apps, Settings and Help use the existing core state and permissions. An embedded build is also available at `/business/`; the original developer dashboard remains at `/`.

## Run and build

```sh
cd business-ui
npm ci
npm run dev                 # http://127.0.0.1:3100; connects to https://localhost:8824
npm run build:amux          # writes crates/amux-dashboard/static/business
npm run typecheck
npx playwright install chromium
npm run test:e2e -- --project=desktop
npm run build:amux
npm run test:e2e -- --project=embedded
npm run test:e2e:live        # requires the development UI and local Amux
```

`AMUX_SERVER_URL` and optional `AMUX_AUTH_TOKEN` configure the loopback development bridge. Never put a token in a `VITE_` variable. TLS certificate exceptions apply only to the loopback development server. Production uses relative native Amux APIs, the current browser's authentication, and the same guarded bootstrap as the developer dashboard. There is no production proxy to a second backend.

Build the frontend before compiling Amux: RustEmbed packages the generated assets into the normal Amux binary. Commit both source and generated `static/business` assets. To test the deployed surface:

```sh
AMUX_LIVE_URL=https://localhost:8824 AMUX_LIVE_PATH=/business/ npm run test:e2e:live
```

The optional Sites/Vinext build (`npm run build`) can use an authenticated reachable `AMUX_SERVER_URL`; a hosted site cannot reach a customer's localhost. Use `npm run dev` for this installation's standalone local frontend.

## Same engine, different surface

| Business surface | Existing Amux primitive |
| --- | --- |
| Work, exception details, reviewer notes | Board items, revision checks, artifacts, verifications |
| Automation definition | Namespaced preferences composing existing sessions, schedules and apps |
| Pause/resume a scheduled run | Existing schedule PATCH |
| Connected apps and reconnect | Connector health and OAuth endpoints |
| Email and permission decisions | Frozen email proposals and one-time grants; native approval authority |
| Natural-language request | Durable backlog item, surfaced immediately as a work artifact |
| Activity and results | Recorded board transitions and evidence; bounded snapshot |

`server/business.ts` is a projection/transport adapter, shared by development and the embedded browser build. It is not an execution engine or a security boundary. The server remains authoritative. Workflow records live under `business_ui.workflows.v1.<workflow-id>`; the aggregate prototype key is read for compatibility. Independent records prevent concurrent creation from replacing the collection. Display-name uniqueness is advisory; record IDs are unique.

An automation composes existing operations. Saving its definition does not create a worker, grant access, install a connector, or change its schedules. Requests enter backlog for planning; the assistant does not claim to have reasoned about or executed them. Business instructions and expected outcomes accompany work created within the selected workflow. They do not constitute deterministic verification or alter independently created tasks.

## Review boundaries

Email approval requires an explicit review checkbox and complete server-supplied message metadata. Older servers, messages over the preview bound, and proposals with attachments that cannot be inspected are not approvable here. Rejecting requests a new proposal; editing a frozen payload is not supported. A successful approval response means the core accepted the action, not that delivery or a business postcondition has been verified.

The server exposes full body review metadata while retaining the legacy short preview. Attachment bytes and paths are not exposed. Existing expiry, ownership and one-time consumption checks remain in Amux. Permission grants keep the existing server authority; this surface does not introduce a new generic safe-action mechanism.

## Honest results and current limits

- Work is a bounded snapshot (2,000 items, up to 500 terminal items). Counts are scoped to loaded work, not an accounting ledger or complete all-time totals.
- Completed and verified remain distinct. Time saved and dollar ROI stay unavailable until measured by the backend.
- Business defaults to all workspace work until reusable workflows are configured. It never injects sample customers into the live workspace.
- Trigger installation, shadow evaluation, new connector action catalogs, durable business memory and deterministic business verification remain roadmap work.
- Connection failures and partial data are explicit. Existing rows stay visible during transient refresh failures.
- A reachable server reporting HTTP 503 with health status `degraded` stays connected, with an explicit warning about unavailable or delayed operations. It does not ask the operator to reconnect or disable all work merely because a health probe failed. Unknown health errors and transport failures remain unavailable; core permissions still govern every action.
- The UI is responsive and keyboard accessible; automated text-resize and layout checks are not a full accessibility certification.

## UI sources

Uses Inter Variable, Lucide, shadcn/Base UI controls and the official [assistant-ui Elements](https://www.assistant-ui.com/elements) registry: composer, artifact, timeline and an adapted approval card. The assistant panel uses the real assistant-ui external-store runtime, thread, messages and copy action. Approval copy and controls were adapted to business decisions; it does not fabricate shell execution or offer permanent access.

Registry source: `https://r.assistant-ui.com/`. The generated component sources are retained in `components/assistant-ui`; unused catalog elements are available for later evidence and connection views. The application follows the supplied Amux Business typography, semantic colors, navigation, spacing and operational hierarchy.

## Verification

Playwright's fixture server contains synthetic records only. Tests cover creation/reload persistence, evidence, revision-aware notes, assistant-to-board capture, approval confirmation and incomplete/expired refusal, connector checks, schedule control, workflow composition, mobile navigation, text resizing and transport restrictions. The separate live test creates one uniquely titled backlog item, verifies its persisted note against the native API and archives that same item in `finally`. It never releases a live approval or changes an existing schedule.

Latest local validation (2026-09-09): 13/13 fixture browser tests; live create/note/archive round trip; 4/4 email approval and 9/9 static-serving Rust tests; TypeScript and both production builds; npm audit: 0 vulnerabilities. The native service worker passes Business and health requests through to the network so the developer shell cache cannot replace the Business page.

Production packaging regression: the compiled bundle suite now covers the 12 applicable UI flows independently of Vinext development. The development-gateway restriction test stays in the desktop project; native authorization is verified in Rust. React/ReactDOM are deduplicated, Sonner is a declared dependency, and the committed lockfile makes clean installs reproducible. Live mutation tests require the explicit `test:e2e:live` script.
