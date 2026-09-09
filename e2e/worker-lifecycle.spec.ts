// The full worker lifecycle, end to end, through the surfaces a person uses.
//
// WHY THIS EXISTS. Every other spec here pins one surface. On 2026-09-09 a
// terminal-renderer change passed 81 scenarios and still broke the product in
// five separate ways within an hour of reaching Ethan, because nothing walked
// a worker from creation to deletion and looked at what a human would see.
// The gap was not coverage of parts; it was that no test owned the WHOLE.
//
// It also pins the BOARD COLUMN GATES, which are the part most likely to rot
// quietly: a gate that stops refusing is indistinguishable from a gate that
// was never there, and the board's whole value is that `done` and `verified`
// mean something. Each column is checked twice — that a bare move is refused,
// and that an INCOMPLETE or FABRICATED acknowledgement is refused too. The
// second half is the one that matters. A gate you can satisfy by sending the
// right-shaped noise is decoration.
//
// Self-cleaning: the worker and card are uniquely named and removed in
// `finally`, so a failed run does not leave a lane or a card behind.
import { test, expect, Page } from '@playwright/test';

async function appToken(page: Page): Promise<string> {
  await page.goto('/');
  const token = await page.evaluate(() => (window as any)._AMUX_AUTH_TOKEN as string);
  expect(token, 'served bootstrap must provide the API token').toBeTruthy();
  return token;
}

test('a worker goes create → run → prompt → peek → delete, and the board gates hold', async ({ page, request }, testInfo) => {
  const token = await appToken(page);
  const auth = { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' };
  const worker = `e2e-life-${testInfo.project.name}-${Date.now()}`;
  let card = '';

  try {
    // ── CREATE, through the real dialog, on a named model ──────────────────
    await page.addInitScript(() => { try { localStorage.setItem('amux_walkthrough_done', '1'); } catch (e) {} });
    await page.goto('/');
    await page.waitForFunction(() => typeof (window as any).fetchSessions === 'function');
    await page.click('#add-btn');
    await page.locator('text=New worker').first().click();
    await page.waitForSelector('#create-name', { state: 'visible' });
    // The model list loads async; selecting before it lands silently picks nothing.
    await expect.poll(async () => page.evaluate(
      () => (document.getElementById('create-model') as HTMLSelectElement).options.length)).toBeGreaterThan(1);
    const sonnet = await page.evaluate(() => [...(document.getElementById('create-model') as HTMLSelectElement).options]
      .map(o => o.value).find(v => /sonnet/i.test(v)) || '');
    expect(sonnet, 'a Sonnet model must be offered in the create dialog').toBeTruthy();
    await page.fill('#create-name', worker);
    await page.fill('#create-dir', process.env.AMUX_E2E_DIR || '/tmp');
    await page.selectOption('#create-model', sonnet);
    await page.locator('#create-overlay button.primary:has-text("Create")').click();

    // The chosen model reaches the PROCESS as a launch flag. Assert the flag,
    // not the `model` field: that one reflects the live harness report and is
    // empty until the lane reports in, so asserting it here fails for a
    // reason that has nothing to do with the dialog.
    await expect.poll(async () => (await request.get(`/api/sessions/${worker}`, { headers: auth })).status(),
      { timeout: 30_000 }).toBe(200);
    const cfg = await (await request.get(`/api/sessions/${worker}`, { headers: auth })).json();
    expect(cfg.flags || '', 'the selected model must reach the launch flags').toContain('sonnet');

    // ── THE WORKER IS VISIBLE WHERE A PERSON LOOKS ────────────────────────
    await page.reload();
    await page.waitForFunction(() => typeof (window as any).fetchSessions === 'function');
    await expect(page.locator(`text=${worker}`).first()).toBeVisible({ timeout: 30_000 });

    // ── PEEK PAINTS, AND PAINTS PLAINLY ───────────────────────────────────
    // The three classes below are the 2026-09-09 regressions. They are pinned
    // by ABSENCE because each one was a wrapper that should not exist at all;
    // asserting on rendered text would have passed through every one of them.
    await page.evaluate((n) => (window as any).openPeek(n), worker);
    await expect.poll(async () => page.evaluate(
      () => (document.getElementById('peek-body') as HTMLElement).textContent!.length),
      { timeout: 30_000 }).toBeGreaterThan(0);
    expect(await page.locator('.peek-code-row, .peek-code-split').count(),
      'plain terminal output must not be re-rendered as a split diff').toBe(0);
    expect(await page.locator('.peek-output-controls .scroll-lock-badge').count(),
      'the scroll-lock badge must not sit in the toolbar layout flow — it resizes the row and the view jumps').toBe(0);

    // The terminal box must not move while output streams. A show/hide element
    // in the toolbar's flow is exactly what made the view bounce.
    const geom = async () => page.evaluate(() => {
      const r = document.getElementById('peek-body')!.getBoundingClientRect();
      return `${Math.round(r.top)}x${Math.round(r.height)}`;
    });
    const seen = new Set<string>();
    for (let i = 0; i < 8; i++) { seen.add(await geom()); await page.waitForTimeout(400); }
    expect([...seen], 'the terminal box must hold still while output streams').toHaveLength(1);
    await page.evaluate(() => (window as any).closePeek());

    // ── BOARD COLUMN GATES ────────────────────────────────────────────────
    // `backlog` is unbounded on purpose, so a probe card always has a home.
    const made = await request.post('/api/board', {
      headers: auth, data: { title: `[e2e] lifecycle gate probe ${worker}`, session: worker, status: 'backlog' },
    });
    expect(made.ok(), 'backlog must always accept a card').toBeTruthy();
    card = (await made.json()).id;

    const statuses = await (await request.get('/api/board/statuses', { headers: auth })).json();
    const move = (data: object) => request.patch(`/api/board/${card}`, { headers: auth, data });
    const statusOf = async () => (await (await request.get(`/api/board/${card}`, { headers: auth })).json()).status;

    for (const col of statuses.filter((s: any) => (s.gate || []).length > 0)) {
      // 1. A bare move is refused, and the refusal names the way through.
      const bare = await move({ status: col.id });
      expect(bare.status(), `${col.id} must refuse a move that acknowledges nothing`).toBe(409);
      const why = await bare.json();
      expect(JSON.stringify(why), `${col.id}'s refusal must tell the caller how to comply`)
        .toMatch(/cli|how_to_ack|how_to_fix/);
      expect(await statusOf(), `${col.id} must not have moved`).not.toBe(col.id);

      // 2. A WRONG acknowledgement is refused. This is the half that decides
      //    whether the gate is real: an exact-match check cannot be satisfied
      //    by sending an array of the right shape.
      const fake = await move({ status: col.id, gate_checked: ['not a real criterion'] });
      expect(fake.status(), `${col.id} must refuse a fabricated acknowledgement`).toBe(409);
      if (col.gate.length > 1) {
        const partial = await move({ status: col.id, gate_checked: [col.gate[0]] });
        expect(partial.status(), `${col.id} must refuse a partial acknowledgement`).toBe(409);
      }
      expect(await statusOf(), `${col.id} must still not have moved`).not.toBe(col.id);
    }

    // 3. The honest path works. `review` is the check: `done` additionally
    //    demands an asset link and `verified` is owner-gated, so neither can
    //    stand for "a complete acknowledgement is accepted".
    const review = statuses.find((s: any) => s.id === 'review');
    const ack = await move({ status: 'review', gate_checked: review.gate });
    expect(ack.ok(), 'a complete, exact acknowledgement must be accepted').toBeTruthy();
    expect(await statusOf()).toBe('review');

    // `done` refuses even a complete gate ack with no artifact to point at.
    const done = statuses.find((s: any) => s.id === 'done');
    const noAsset = await move({ status: 'done', gate_checked: done.gate });
    expect(noAsset.status(), 'done must refuse a card that names no artifact').toBe(409);
    expect((await noAsset.json()).code).toBe('done_requires_asset_link');

    // ── DELETE, and no operation strands in the outbox ────────────────────
    const del = await request.delete(`/api/sessions/${worker}`, { headers: auth });
    expect(del.ok(), 'DELETE must be accepted').toBeTruthy();
    await expect.poll(async () => (await request.get(`/api/sessions/${worker}`, { headers: auth })).status(),
      { timeout: 30_000 }).toBe(404);

    await page.reload();
    await page.waitForFunction(() => typeof (window as any).fetchSessions === 'function');
    await expect(page.locator('text=/Unsaved changes/')).toHaveCount(0);
  } finally {
    if (card) await request.delete(`/api/board/${card}`, { headers: auth }).catch(() => {});
    await request.delete(`/api/sessions/${worker}`, { headers: auth }).catch(() => {});
  }
});
