// Phase 5 browser-side golden scenarios (RR-0083 offline mode, RR-0087
// real-time convergence), run against the REAL extracted 1.4MB dashboard
// served by the Rust server — not a synthetic harness page.
//
// ── Platform truths this spec is written against (measured, not assumed) ──
//
// 1. SSE never establishes. The SPA authenticates its EventSource with
//    `?_token=` (app.js `_authUrl`), but the Rust auth middleware accepts only
//    `Authorization: Bearer` or `?token=` (crates/amux-server/src/api/auth.rs
//    strips the literal prefix "token=", which `_token=` does not match).
//    Verified live: GET /api/events?_token=<valid> → 401; ?token=<valid> →
//    stream. So the dashboard's EventSource 401s, onerror fires 3x, and the
//    SPA's own sanctioned degradation — enablePollingFallback(), which runs
//    fetchSessions+fetchBoard every 5s — becomes the live-update transport.
//    This spec waits for `_pollTimer` (never cleared once set) as the "live
//    update machinery is active" signal, and the RR-0087 "kill SSE
//    mid-stream" step uses context.setOffline(true→false) — the task's
//    sanctioned fallback — because there is no live EventSource reachable
//    from page context to close.
//
// 2. Delta sync is a protocol no-op. The SPA's _runDeltaSync speaks the
//    Python contract (`/api/sync?since=` → {issues, statuses, ...}); the Rust
//    /api/sync returns {rev, events:[...]}. The client finds no `issues` key
//    and applies nothing. Convergence rides fetchBoard (full list), not the
//    delta path.
//
// 3. `boardItems`, `online`, `offlineQueue`, `_pollTimer` are top-level `let`
//    bindings — they live in the global lexical environment, NOT on `window`.
//    String-form page.evaluate resolves them exactly like the page's own
//    code does; `(window as any).boardItems` would be undefined.
//
// 4. /api/sessions (alias → /api/workers) returns {items:[],...} while the
//    SPA expects a bare array; fetchSessions throws internally (caught) every
//    poll. Board convergence is unaffected — fetchBoard parses fine — but it
//    is why this spec never waits on session data.
import { test, expect, Page, APIRequestContext } from '@playwright/test';

// ---- helpers ---------------------------------------------------------------

/** Bearer token as the SPA received it from the served bootstrap. */
async function appToken(page: Page): Promise<string> {
  const tok = await page.evaluate(() => (window as any)._AMUX_AUTH_TOKEN as string);
  expect(tok, 'served bootstrap must inject a non-empty auth token').toBeTruthy();
  return tok;
}

function authHeaders(token: string): Record<string, string> {
  return { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' };
}

/** Load '/' and wait for the app shell + its API layer to be live. */
async function settle(page: Page): Promise<void> {
  await page.goto('/');
  await expect(page.locator('#conn-status').first()).toBeAttached();
  await page.waitForFunction(() => typeof (window as any).apiCall === 'function');
  // The service worker cannot register on this origin (self-signed cert), so
  // the real app shows its "Offline mode is OFF" failure bar (#sw-fail-bar) —
  // a fixed bottom overlay that on a 375px viewport covers bottom-anchored
  // controls (it blocked the board modal's Save button on mobile). Dismiss it
  // exactly as a user would — via its own × close button — whenever it
  // appears in the way of an action.
  await page.addLocatorHandler(page.locator('#sw-fail-bar'), async (bar) => {
    await bar.locator('button').last().click();
  });
}

/** The SPA's offline outbox, straight from localStorage (its storage of record). */
function outbox(page: Page): Promise<Array<{ url: string }>> {
  return page.evaluate(
    "JSON.parse(localStorage.getItem('amux_offline_queue') || '[]')",
  ) as Promise<Array<{ url: string }>>;
}

/** In-memory board state — the `boardItems` lexical global. */
function boardTitles(page: Page): Promise<string[]> {
  return page.evaluate(
    "(typeof boardItems === 'undefined' ? [] : boardItems).map(i => String(i.title))",
  ) as Promise<string[]>;
}

/** True once the SPA's polling fallback is running (see header note 1). */
function pollActive(page: Page): Promise<boolean> {
  return page.evaluate(
    "typeof _pollTimer !== 'undefined' && _pollTimer != null",
  ) as Promise<boolean>;
}

async function serverTitles(request: APIRequestContext, token: string): Promise<string[]> {
  const res = await request.get('/api/board?archived=all&done_limit=0', {
    headers: authHeaders(token),
  });
  expect(res.status()).toBe(200);
  const items = (await res.json()) as Array<{ title: string }>;
  return items.map((i) => i.title);
}

// ---- RR-0083: offline queue and replay -------------------------------------
//
// Offline → 3 board cards created THROUGH THE REAL UI (board tab → “+ New
// issue” → title → Save) → SPA queues them in its outbox → reconnect → the
// SPA's own 'online' listener replays via runSyncBanner → server holds each
// exactly once, queue drained, client converges.

test('golden_offline_queue_and_replay', async ({ page, request }, testInfo) => {
  test.setTimeout(120_000);
  await settle(page);
  const token = await appToken(page);
  const prefix = `e2e-off-${testInfo.project.name}-w${testInfo.workerIndex}r${testInfo.retry}-${Date.now()}`;
  const titles = [1, 2, 3].map((n) => `${prefix} card ${n}`);

  // Real UI: open the board view before going offline.
  await page.click('#tab-board');
  await expect(page.locator('#board-view')).toBeVisible();

  // ---- go offline (fires the window 'offline' event → setOnline(false)) ---
  await page.context().setOffline(true);
  await expect(page.locator('#conn-status').first()).toHaveText(/Offline|pending/, {
    timeout: 10_000,
  });

  // ---- create 3 cards through the SPA's own add flow ----------------------
  for (const title of titles) {
    await page.click('.board-new-btn');
    await expect(page.locator('#board-edit-overlay')).toHaveClass(/active/);
    await page.fill('#be-title', title);
    await page.click('.be-save');
    await expect(page.locator('#board-edit-overlay')).not.toHaveClass(/active/);
  }

  // ---- offline queue mechanics: outbox holds exactly our 3 mutations ------
  await expect.poll(async () => (await outbox(page)).length, { timeout: 10_000 }).toBe(3);
  const queued = await outbox(page);
  for (const op of queued) expect(op.url).toContain('/api/board');

  // The pending UI reflects the queue: status pill + offline banner.
  await expect(page.locator('#conn-status').first()).toHaveText('3 pending');
  await expect(page.locator('#offline-banner')).toHaveClass(/active/);
  await expect(page.locator('#offline-banner-title')).toContainText('3 ops');

  // The queue really is local: nothing reached the server yet.
  const during = await serverTitles(request, token);
  for (const t of titles) expect(during).not.toContain(t);

  // ---- reconnect: the SPA's own resume path does the replay ---------------
  await page.context().setOffline(false);
  // Playwright's Chromium emulation fires the window 'online' event, which is
  // the SPA's sanctioned trigger (setOnline(true) → runSyncBanner). Belt and
  // suspenders: if the SPA has not flipped its own `online` flag shortly, we
  // dispatch the same event it listens for — and record that we had to.
  const flipped = await page
    .waitForFunction("typeof online !== 'undefined' && online === true", null, {
      timeout: 5_000,
    })
    .then(() => true)
    .catch(() => false);
  if (!flipped) {
    testInfo.annotations.push({
      type: 'note',
      description:
        "native 'online' event did not flip the SPA flag within 5s; dispatched window 'online' manually",
    });
    await page.evaluate("window.dispatchEvent(new Event('online'))");
  }

  // Queue drains through runSyncBanner…
  await expect.poll(async () => (await outbox(page)).length, { timeout: 30_000 }).toBe(0);
  // …and the sync banner reported the replay in the UI.
  await expect(page.locator('#sync-title-text')).toContainText(/synced/, { timeout: 15_000 });

  // ---- server holds every card exactly once (RR-0083: no duplicates) ------
  await expect
    .poll(async () => {
      const all = await serverTitles(request, token);
      return titles.filter((t) => all.includes(t)).length;
    }, { timeout: 15_000 })
    .toBe(3);
  const finalAll = await serverTitles(request, token);
  for (const t of titles) {
    expect(finalAll.filter((x) => x === t), `no duplicate replay of "${t}"`).toHaveLength(1);
  }

  // ---- client converged too: boardItems carries the replayed cards --------
  await expect
    .poll(async () => (await boardTitles(page)).filter((t) => titles.includes(t)).length, {
      timeout: 20_000,
    })
    .toBe(3);
});

// ---- RR-0087: real-time convergence ----------------------------------------
//
// Two independent contexts. Page 1 creates 5 cards via authenticated API
// POSTs; page 2 converges through the SPA's live-update transport (polling
// fallback — see header note 1). Then page 2's transport is killed mid-stream
// (setOffline — the sanctioned fallback given SSE never establishes), 3 more
// cards are created during the gap, and page 2 must converge again after
// reconnect. Final state: both pages hold the identical set.

test('golden_realtime_convergence', async ({ browser, baseURL, request }, testInfo) => {
  test.setTimeout(120_000);
  const ctxOpts = {
    baseURL,
    ignoreHTTPSErrors: true, // manual contexts do not inherit project `use`
    viewport: testInfo.project.use.viewport ?? undefined,
  };
  const ctx1 = await browser.newContext(ctxOpts);
  const ctx2 = await browser.newContext(ctxOpts);
  try {
    const page1 = await ctx1.newPage();
    const page2 = await ctx2.newPage();
    await settle(page1);
    await settle(page2);
    const token = await appToken(page1);
    const prefix = `e2e-rt-${testInfo.project.name}-w${testInfo.workerIndex}r${testInfo.retry}-${Date.now()}`;

    // Page 2 on the board view so convergence is also visible in the DOM.
    await page2.click('#tab-board');
    await expect(page2.locator('#board-view')).toBeVisible();

    // Wait until each page's live-update machinery is actually running
    // (poll timer armed after the SSE 401/retry cycle, ~6s after load).
    await expect.poll(() => pollActive(page1), { timeout: 30_000 }).toBe(true);
    await expect.poll(() => pollActive(page2), { timeout: 30_000 }).toBe(true);

    // ---- phase 1: 5 rapid creates; page 2 must converge within 10s --------
    const first = [1, 2, 3, 4, 5].map((n) => `${prefix} rapid ${n}`);
    for (const title of first) {
      const res = await request.post('/api/board', {
        headers: authHeaders(token),
        data: { title, type: 'chore', status: 'todo' },
      });
      expect(res.status()).toBe(201);
    }
    const mine = (all: string[]) => all.filter((t) => t.startsWith(prefix));
    await expect
      .poll(async () => mine(await boardTitles(page2)).length, { timeout: 10_000 })
      .toBe(5);
    // Rendered UI carries them too (not just the in-memory array).
    for (const title of first) {
      await expect(page2.locator('#board-view')).toContainText(title, { timeout: 5_000 });
    }

    // ---- phase 2: kill page 2's transport mid-stream ----------------------
    // No live EventSource exists to close (SSE 401s — header note 1), so the
    // sanctioned fallback applies: a real network gap via setOffline.
    await ctx2.setOffline(true);
    await expect(page2.locator('#conn-status').first()).toHaveText(/Offline|pending/, {
      timeout: 10_000,
    });

    const second = [6, 7, 8].map((n) => `${prefix} gap ${n}`);
    for (const title of second) {
      const res = await request.post('/api/board', {
        headers: authHeaders(token),
        data: { title, type: 'chore', status: 'todo' },
      });
      expect(res.status()).toBe(201);
    }
    // The gap is real: page 2 cannot have seen the new cards while offline.
    await page2.waitForTimeout(1_500);
    expect(mine(await boardTitles(page2)).length).toBe(5);

    // ---- reconnect: resume/refetch path must converge within 15s ----------
    await ctx2.setOffline(false);
    await expect
      .poll(async () => mine(await boardTitles(page2)).length, { timeout: 15_000 })
      .toBe(8);

    // Page 1 (never disturbed) also holds all 8.
    await expect
      .poll(async () => mine(await boardTitles(page1)).length, { timeout: 15_000 })
      .toBe(8);

    // ---- final: identical state on both pages, exactly once on the server -
    const t1 = mine(await boardTitles(page1)).sort();
    const t2 = mine(await boardTitles(page2)).sort();
    expect(t1).toEqual(t2);
    expect(t1).toHaveLength(8);
    const all = await serverTitles(request, token);
    for (const t of [...first, ...second]) {
      expect(all.filter((x) => x === t), `exactly one server card for "${t}"`).toHaveLength(1);
    }
  } finally {
    await ctx1.close();
    await ctx2.close();
  }
});
