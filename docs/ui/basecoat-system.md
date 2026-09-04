# AMUX Basecoat UI system

This document is the source-of-truth inventory and migration contract for the AMUX dashboard. It was derived from the shipped `index.html`, every view dispatched by `switchView`, embedded workspace routes, worker-detail tabs, and the dynamic menu/modal renderers in `app.js`.

## Product shell

AMUX is a single static application shell with three navigation layers:

1. The application header: brand, connection state, notifications, organization switcher, rate-limit state, active workers, create/connect actions, and settings.
2. The primary tab strip: the user-reorderable top-level pages.
3. Contextual shells: the worker overlay, file/MDAI viewers, workspace panels, and full-screen utilities.

The Basecoat migration keeps those information-architecture decisions intact. It standardizes the visual and interaction language beneath them.

## Page inventory

### Primary navigation

| Page | Main regions | Local views and interactions |
| --- | --- | --- |
| Workers | Search/filter row, active filters, group scope, offline queue, pinned notes, live worker cards, archived workers | List and tile layouts; sorting and frozen order; expand/collapse; per-card composer; card menu; branch popover; worker create, duplicate, connect, archive, and delete flows |
| Board | Query/filter builder, owner switch, view switch, export, nudges, status lanes | Dense list, group-by-worker, group-by-status; saved views; unclaimed work; statistics; drag/drop cards; lane menus; card context menu; add/edit/detail surfaces |
| Groups | Group summary rows | Expand/collapse membership; workers, board, costs, memory/configuration, and group editing links |
| Calendar | FullCalendar canvas | Month/week/day/list navigation; event creation and editing; schedule/board integration |
| Scheduler | Search, user schedules, system jobs, recent runs, audit history | New/edit schedule; loop/routine/once modes; run-now, enable/disable, history and audit disclosures |
| Files | File-manager toolbar, breadcrumbs, bookmarks, search, sortable columns, status bar | Name filtering and contents search; hidden files; upload; new file/folder/MDAI; offline cache; Finder; list and library views; file menus |
| MDAI | Computed-node toolbar and node list | Create/refresh; node viewer; raw file; run DAG; run history; model/configuration menus |
| Proxies | Intro, quota/status, proxy cards | Create/edit proxy dialog; start/stop; public URL copy; scheme/port configuration |
| Email | Priority explanation, inference provenance, mailbox subtabs, folder filters, message list | Refresh/re-learn; Gmail accounts and threads; ranking themes; reply/archive-style message actions |
| Connectors | Account summary, connector registry, add-connector form | API-key and OAuth definitions; global/group/worker scope; secret setup; tests and account rows |
| Logs | Category filter bar, search, page subtabs | Activity, raw logs, statistics, and health; raw-line filtering; auto refresh |
| Workspace | GridStack canvas and layout toolbar | Add worker/view/note tiles; resize/reorder; presets; profiles; save/share layouts; fullscreen; per-tile terminal/composer |
| Messages | Messages/trends mode switch, search, worker/date/kind filters, timeline | Deep-history search; multi-select and resend; row menus; locate in worker; load older; trends by 7/14/30 days |
| Skills | Search, count, sectioned skill cards | New/edit/delete skill; content editor |
| Database | Table sidebar, data/structure/query modes | Table filter; pagination; row grid; schema; SQL editor; read-only default and explicit `wb_*` write toggle |
| Map | Collapsible pin sidebar and map canvas | Geocoding; groups; pin search/sort; import/export; drop pin; locate; saved view; pin and group dialogs |
| Metrics | Worker sidebar and main metrics pane | System and Disk Cleanup modes; resource cards, tables, speed tests, reclaim categories/actions |
| Cost | Period and group filters, totals and breakdowns | Today/7/30/90-day windows; worker/group attribution; cost/token cards and rows |
| Torrents | Add row, settings panel, transfer list | Magnet/URL add; destination folder; progress/status; pause/resume/remove and browse actions |
| Terminal | Connection toolbar and xterm canvas | Local shell or SSH profile; connect/disconnect; connection status; clear; font sizing |
| Browser | Navigation, status, interaction, agent, viewport, and inspector rows | Profile/backend selection; back/go/live/snapshot; type and key controls; element list; autonomous task; console/network/error inspector |

### Additional and embedded surfaces

| Surface | Reachability | Local views |
| --- | --- | --- |
| Graph | Present in `switchView` and DOM; not currently in the primary tab markup | Notes/Fleet graph switch, vault import, search/filter sidebar, pan/zoom/reset/edge controls, node detail drawer |
| Journal | Present in DOM, embed allowlist and dispatcher; not currently in primary tab markup | Entry list/editor, Calendar, Media, and Map subtabs; prompts/configuration; attachments |
| Habits | Present in DOM, embed allowlist and dispatcher; not currently in primary tab markup | Habit cards, seven-day history, streak, add/edit/delete, daily completion |
| Trends | Nested in Messages | Weekly task-theme summary, needs-you state, worker/card links, historical digest selector |
| File library | Nested in Files | Search; author, format, tag, and sort facets; cover grid |
| Workspace embed | `?embed=<view>` | Isolated copy of a top-level view in a grid tile |
| Worker embed | `?peekEmbed=<worker>` or `?embed=peek:<worker>` | Full worker detail inside a workspace tile |
| Chrome-style app tabs | Persistent browser-like strip above the shell | Add, switch, rename, collapse, and close app frames |
| DevTools | Docked utility panel | Console, Network, and Info tabs plus a JavaScript expression input |
| Walkthrough | Guided overlay | Spotlight, tooltip, next/back/close, and temporary tab reveal |

## Worker detail inventory

The worker overlay is a second application shell. Its header contains worker identity/status/model, current task, find-in-terminal controls, message navigation, file/focus actions, and close. The tab strip is user-reorderable; Terminal is pinned.

| Tab | Contents |
| --- | --- |
| Terminal | Live terminal output, terminal search/navigation, subagent strip, composer, attachment chips, saved/history messages, slash commands, voice popup, fullscreen composer, optional file-browser split |
| Translate | Plain-language summary or a user-defined transformation prompt; refresh and link back to Terminal |
| Steering | Pending steering queue, sent-history disclosure, edit/remove and send controls |
| Schedules | Worker-scoped copy of Scheduler with search, new schedule, schedule rows, and recent runs |
| Configurations | Effective global/group/worker capability values, provenance, writeability, configuration editor, worker preference controls |
| Messages | Worker-scoped message history, search, kind filters, selection/resend, row actions |
| Dictation | Record/stop, transcript, dictionary and replacement controls, API-key/setup state |
| Read alouds | Generated audio history, play/download/delete controls and retention state |
| Board | Worker-scoped board in list or lane view, shared query language, export and item details |
| Cost | Worker token/cost totals and task attribution |
| Transcript | Structured JSONL conversation timeline with plan/tool/message rows |
| Commits | Commit list sidebar and commit-detail/diff pane |
| Memory | Edit, Preview, Global, and Inherited subtabs; Markdown editor and scope/provenance messaging |
| Worktree | Branch/action header, collapsible file tree, working/all-changes filter, diff viewer, push and PR actions |
| Logs | All/Errors/Slow filters, request-log search, summary, and request rows |

## Menu, dropdown, popover, and disclosure inventory

### Global shell

- Notification panel and notification-policy controls.
- Organization/workspace switcher.
- Rate-limit bulk action entry.
- Active-worker dropdown.
- Add menu: New worker, Connect tmux, Connect iTerm2, Orchestrate, Bulk actions.
- Settings popover with Account, Workers, Alerts, Connect, and Device tabs.
- Primary-tab customizer with hide/show, drag reorder, save/load/share/delete presets.

### Workers and worker detail

- Worker card menu: task label, peek, read, browse, info, pin, rename, provider/model/effort, YOLO, isolated mode, description, groups, auto-drain, cross-group reach, directory, restart, stop, clear, duplicate, new conversation, share, archive, delete.
- Branch popover and worker autocomplete menus.
- Worker-detail More menu, composer menu, slash-command autocomplete, attachment actions, selection actions, message row menus, and worker-tab customizer.
- Steering-history, inherited-memory, schedule-runs, and template disclosures.

### Board and schedules

- Filter-builder popover and active-filter chips.
- Saved board views.
- Nudges scope popover.
- Lane menu and card context menu.
- Board card details, status buttons, preview/edit tabs, artifact links, history and evidence.
- Schedule recent-runs and audit disclosures; schedule mode and worker/model pickers.

### Files, MDAI, and workspace

- File toolbar overflow menu, per-row file menus, Explorer row menus, file-view action menu.
- File Preview/Edit/Raw tabs; Markdown find, read-aloud, copy/path/link, teleprompter, download.
- MDAI action menu, body tabs, node picker and dependency actions.
- Workspace profile chips, saved-layout menu, note menu, add-panel picker, and tile menus.

### Remaining pages

- Messages kind filters, row action menus, bulk selection bar, trends period selector.
- Logs categories and Activity/Raw/Stats/Health tabs.
- Database Data/Structure/Query tabs, table picker, pager and write confirmation.
- Map geocoder results, group chips, sort/export menu, pin/group dialogs and node side panel.
- Metrics mode switch and cleanup category cards.
- Browser profile/backend selects, Elements panel and Console/Network/Errors inspector tabs.
- Journal List/Calendar/Media/Map tabs, tag filters, editor actions and configuration dialog.
- DevTools Console/Network/Info tabs.

## Overlay and dialog inventory

- API-key setup and cloud upgrade.
- Generic prompt, confirm, and alert dialogs.
- Create worker; connect tmux; connect iTerm2.
- Worker detail and subagents.
- Edit field and queue.
- Board add/edit and full-screen board detail.
- Schedule editor and calendar event editor.
- Voice fleet orchestrator: Record, Transcript, Routing review.
- Command history, filters, saved messages, and channel drawer.
- Scope/configuration editor and bulk actions.
- Skill editor.
- Proxy editor.
- Map pin and map group editors.
- File viewer and MDAI node viewer. Directory browsing always routes through
  the canonical Directory Explorer mount; there is no separate explorer dialog.
- Video player, teleprompter, TTS, message lookup, chip picker, and selected-terminal-text popover.
- About/branding/server switcher/debug.
- Sync status, offline status, toast and notification banners.

## Cross-context capability convergence

A capability is implemented once and receives scope/context as data. A compact
mount may omit secondary chrome, but it cannot own a second renderer, visual
language, state model, or interaction contract. These are the canonical AMUX
components and every place they are mounted:

| Canonical capability | Contexts | Shared implementation contract |
| --- | --- | --- |
| Directory Explorer | Files page; worker directory-path click; worker More → split file browser | `_directoryExplorerBreadcrumb`, `_directoryExplorerFetch`, `_directoryExplorerRender`, `_directoryExplorerBuildRow`, and `_directoryExplorerToggleExpand`. All contexts share sorting, rows, inline expansion, menus, pointer/touch opening, offline fallback, empty/error states, and `openFilePreview`. Only the mount and navigation callback differ. |
| File Viewer | Files page; worker full-page directory; worker split directory; terminal-path links; message attachments | `openFilePreview` is the single dispatcher for text, Markdown, CSV, image, PDF, video, audio, spreadsheet, MDAI and edit/raw modes. A directory context never implements an alternate preview. |
| Board Surface | Global Board; group-scoped Board; worker Board tab | `_renderBoardColumnsInto` owns lanes, cards, buckets, empty states, gates and drag behavior; `_issueRowHTML` owns dense list rows. Scope controls filtering and contextual chrome only. |
| Message Timeline | Global Messages; worker Messages; command-history dialog | `_cmdHistItemHTML` owns every delivered-message row and its menu. Each mount is stamped `data-component="message-timeline"`; context supplies selection, resend, destination and locate behavior. |
| Scheduler Surface | Global Scheduler; worker Schedules tab | `renderScheduler(opts)` owns cards, grouping, disabled state, recent runs and filtering. Worker scope is an option, not a forked view. |
| Terminal Surface | Global Terminal page; Workspace shell tiles | `_createTerminalSurface` owns xterm configuration, ANSI theme, typography, link addon, fitting and semantics. Context owns PTY lifecycle and surrounding toolbar. |
| Worker Session Console | Worker peek; workspace worker embed; restored/deep-linked peek | The same worker overlay, tab model, live output renderer and composer are mounted/restored; embeds do not get a parallel worker console. |
| Configuration Scope Editor | Global, group and worker capability layers | The scope editor receives level/name/capability and renders effective value plus provenance from one editor contract. |

Convergence is enforced with `data-component`/`data-ui-component` markers and
asset-contract tests. A new contextual surface must mount an existing component
or extend that component with an explicit option; copying its markup is a failed
migration even if the copy looks identical.

## Standard Basecoat component vocabulary

AMUX uses Basecoat 1.0.2 from locally served, pinned assets. Existing behavior remains in `app.js`; `ui-system.js` supplies semantic enhancement and ARIA synchronization for both static and dynamically rendered DOM.

| AMUX pattern | Basecoat foundation | AMUX contract |
| --- | --- | --- |
| Action | Button / Button Group | `.btn`; `data-variant=primary|secondary|outline|ghost|destructive`; `data-size=xs|sm|default|icon*` |
| Text entry | Input / Input Group / Textarea | `.input`, `.textarea`, `.field`; one focus-ring and invalid-state system |
| Choice | Native Select / Checkbox / Switch / Slider | `.select`, `.checkbox`, `.switch`, `.range`; native form semantics stay authoritative |
| Navigation | Tabs | `[data-ui-tablist]`, `role=tablist`, `role=tab`, `aria-selected`, and controlled panel ids |
| Surface | Card / Item | `.card` and `[data-ui-surface]`; shared radius, border, padding and hover rules |
| Status | Badge / Alert / Progress | `.badge`, `.alert`, progress meters and consistent semantic colors |
| Menu | Dropdown Menu / Popover | `[data-ui-menu]`, `role=menu`, menuitem roles, trigger `aria-expanded`; existing positioning logic is retained where it encodes viewport safety |
| Modal | Dialog / Alert Dialog / Drawer | `[data-ui-dialog]`, `role=dialog`, `aria-modal`; destructive confirmation is visually distinct |
| Loading/zero state | Spinner / Skeleton / Empty | `[data-ui-loading]`, `.skeleton`, `.empty`; zero results and failed measurements remain distinguishable |
| Data | Table / Pagination / Scroll Area | `.table`, pager button groups and consistent sticky headers |
| Navigation context | Breadcrumb / Sidebar | file breadcrumbs and left-side panes share one density and selection model |
| Feedback | Toast / Alert | one toast geometry, banner hierarchy and success/warn/error vocabulary |

## Tokens

Basecoat semantic tokens are authoritative. Legacy AMUX tokens remain aliases during migration so existing renderers cannot drift into a parallel theme.

| Intent | Basecoat token | Legacy alias |
| --- | --- | --- |
| Page | `--background`, `--foreground` | `--bg`, `--text`, `--fg` |
| Raised surface | `--card`, `--card-foreground` | `--card`, `--surface` |
| Floating surface | `--popover`, `--popover-foreground` | menu and overlay surfaces |
| Main action | `--primary`, `--primary-foreground` | `--accent` plus contrast color |
| Secondary action | `--secondary`, `--secondary-foreground` | secondary buttons and inactive tabs |
| Quiet content | `--muted`, `--muted-foreground` | `--dim`, `--muted` |
| Highlight | `--accent`, `--accent-foreground` | hover and selected-row backgrounds |
| Destructive | `--destructive` | `--red` |
| Structure | `--border`, `--input`, `--ring` | borders, form borders, focus state |

AMUX keeps green for healthy/running, amber for waiting/warning, red for failed/destructive, purple for review/steering, cyan for informational automation, and blue for navigation/action. Color is never the sole status signal.

## Layout rules

- Desktop shell: maximum readable width for document-like pages; full-width lanes, terminals, tables, maps and workspaces.
- Toolbar: 40–44 px controls, one compact density, labeled primary action, overflow for secondary actions.
- Tabs: line treatment for page navigation; contained treatment for local mode switches. Horizontal overflow is explicit and keyboard states remain visible.
- Cards: 12 px radius, one border, restrained shadow; title/meta/content/action order is consistent.
- Dialogs: centered content for short forms, full-screen or drawer treatment for long workflows; sticky header/footer when content scrolls.
- Mobile: 44 px touch targets, edge-to-edge content where density matters, bottom-sheet menus where already supported, safe-area padding at viewport edges.
- Terminal/code/diff/video stay intentionally dark in either theme for stable ANSI and syntax contrast.

## Interaction and accessibility rules

- Active tabs expose `aria-selected`; inactive tabs use `tabindex=-1`.
- Menu triggers expose `aria-haspopup` and `aria-expanded`; menu items receive roles without replacing the existing click handlers.
- Overlays expose `role=dialog`, `aria-modal=true`, and an accessible name from their first heading or a stable fallback.
- Icon-only buttons must have `title` or `aria-label`.
- Focus uses Basecoat's ring token everywhere, including legacy controls.
- Disabled/loading controls keep their visible label and publish `aria-busy` where the app already has an in-flight marker.
- Empty, offline, failed, unmeasured, and genuinely zero states remain separate; a style refactor must not collapse their meaning.
- Dynamic DOM is enhanced through one observer and passed to `basecoat.initAll()`; renderers do not need per-feature initialization code.

## Migration coverage contract

The migration is complete only when:

1. Basecoat CSS and JavaScript are pinned and served locally.
2. Every top-level view and worker tab carries the Basecoat semantic enhancement layer.
3. Buttons, fields, selects, textareas, tabs, cards, menus, overlays, badges, tables, alerts, empty states and progress affordances use the shared vocabulary.
4. Dark/light mode updates Basecoat and legacy aliases in the same operation.
5. Dynamically inserted cards, rows, menus and dialogs receive the same enhancement as initial markup.
6. Existing behavior and viewport-positioning logic are not replaced by a second interaction engine.
7. The service worker caches the Basecoat assets and the AMUX UI-system assets.
8. Static, Rust asset-contract, and browser checks cover desktop and phone widths.

## Verification snapshot

Verified 2026-09-04 against the local dashboard and API:

- All 21 primary pages were opened at desktop width. Every page exposed the
  selected top-level tab, an active `tabpanel`, a non-zero surface, and zero
  document-level horizontal overflow.
- All 20 phone-available pages were opened in an isolated 390 × 844 viewport
  (375 px content width with scrollbar). They retained correct tab/panel state,
  non-zero layouts, and zero document-level horizontal overflow. Workspace is
  intentionally hidden at the phone breakpoint and was verified at desktop
  width and through its embedded views.
- The worker shell filled the phone viewport, exposed all 15 worker tabs, and
  retained one selected Terminal panel. Settings, Scheduler, Messages/Trends,
  Logs, Database, Metrics, Calendar, Journal, and Browser inspector subtabs were
  exercised against their controlled panels.
- Directory Explorer was exercised as the global Files page, worker full-page
  route, and worker split mount. All three identify as the same canonical
  `directory-explorer`; the retired Explorer overlay/renderer is absent.
- `dashboard_assets` contains 26 passing contracts for asset order, offline
  caching, complete page coverage, unique DOM ids, accessibility state,
  cross-context convergence, embed restoration, version parity, and Calendar
  initialization. Workspace compilation, JavaScript syntax, SPA lint (zero
  errors), dependency audit, and `git diff --check` also pass.
