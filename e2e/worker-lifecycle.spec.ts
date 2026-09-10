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

for (const modelFamily of ['sonnet', 'haiku']) {
test(`${modelFamily} worker goes create → run → prompt → peek → delete, and the board gates hold`, async ({ page, request }, testInfo) => {
  // First-run worker boot is intentionally part of this journey. WebKit under
  // the full six-worker matrix reached the guarded delete at 30.1s once; the
  // default 30s budget made infrastructure load, not a product assertion, the
  // verdict. Every individual wait below remains tightly bounded.
  test.setTimeout(90_000);
  const token = await appToken(page);
  const auth = { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' };
  const uiToken = await page.evaluate(() => (window as any)._AMUX_UI_TOKEN as string);
  expect(uiToken, 'served bootstrap must provide the human-only destructive guard').toBeTruthy();
  const humanAuth = { ...auth, 'X-Amux-UI-Token': uiToken };
  const worker = `e2e-life-${modelFamily}-${testInfo.project.name}-${Date.now()}`;
  const prompt = `Lifecycle ${modelFamily} delivery probe ${worker}`;
  const workerAuth = { ...auth, 'X-Amux-Worker': worker };
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
    const namedModel = await page.evaluate((family) => [...(document.getElementById('create-model') as HTMLSelectElement).options]
      .map(o => o.value).find(v => new RegExp(family, 'i').test(v)) || '', modelFamily);
    expect(namedModel, `a ${modelFamily} model must be offered in the create dialog`).toBeTruthy();
    await page.fill('#create-name', worker);
    await page.fill('#create-dir', process.env.AMUX_E2E_DIR || '/tmp');
    await page.fill('#create-prompt', prompt);
    await page.selectOption('#create-model', namedModel);
    await page.locator('#create-overlay button.primary:has-text("Create")').click();

    // The chosen model reaches the PROCESS as a launch flag. Assert the flag,
    // not the `model` field: that one reflects the live harness report and is
    // empty until the lane reports in, so asserting it here fails for a
    // reason that has nothing to do with the dialog.
    await expect.poll(async () => (await request.get(`/api/sessions/${worker}`, { headers: auth })).status(),
      { timeout: 30_000 }).toBe(200);
    const cfg = await (await request.get(`/api/sessions/${worker}`, { headers: auth })).json();
    expect(cfg.flags || '', 'the selected model must reach the launch flags').toContain(modelFamily);
    await expect.poll(async () => {
      const history = await (await request.get(`/api/history?session=${encodeURIComponent(worker)}&limit=20`, { headers: auth })).json();
      return JSON.stringify(history);
    }, { message: 'the create-time prompt must be durably attributed to this worker', timeout: 30_000 }).toContain(prompt);

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
      const rect = (selector: string) => {
        const r = document.querySelector(selector)?.getBoundingClientRect();
        return r ? `${Math.round(r.top)}x${Math.round(r.height)}` : '-';
      };
      return {
        body: rect('#peek-body'), header: rect('#peek-overlay > .overlay-header'),
        tabs: rect('#peek-overlay > .peek-tabs'), dir: rect('#peek-overlay > .peek-dir-bar'),
        plan: rect('#peek-plan'), status: rect('#peek-status'), cmd: rect('.peek-cmd-bar'),
        chips: rect('#peek-chips'), classes: document.getElementById('peek-overlay')!.className,
      };
    });
    const samples = [];
    for (let i = 0; i < 8; i++) { samples.push(await geom()); await page.waitForTimeout(400); }
    const seen = new Set(samples.map(s => s.body));
    expect([...seen], `the terminal box must hold still while output streams\n${JSON.stringify(samples, null, 2)}`).toHaveLength(1);
    await page.evaluate(() => (window as any).closePeek());

    // ── BOARD COLUMN GATES ────────────────────────────────────────────────
    // `backlog` is unbounded on purpose, so a probe card always has a home.
    const foreign = await request.post('/api/board', {
      headers: workerAuth, data: { title: `[e2e] forbidden foreign card ${worker}`, session: `${worker}-peer`, status: 'backlog' },
    });
    expect(foreign.status(), 'a worker must not create on another worker\'s board').toBe(403);
    expect((await foreign.json()).code).toBe('cross_board_create_forbidden');

    const made = await request.post('/api/board', {
      headers: workerAuth, data: { title: `[e2e] lifecycle gate probe ${worker}`, session: worker, status: 'backlog' },
    });
    expect(made.ok(), 'backlog must always accept a card').toBeTruthy();
    card = (await made.json()).id;

    const statuses = await (await request.get('/api/board/statuses', { headers: auth })).json();
    // Column rows expose only a column's own override. The enforceable gate is
    // per card (card → worker → group → global → type default), so use the
    // same resolved contract the transition handler tells callers to read.
    // Falling back to `status.gate` keeps custom, non-core columns covered.
    const contract = await (await request.get(`/api/board/contract?card=${card}`, { headers: auth })).json();
    const resolvedGates = contract.card_effective_gates?.gates || {};
    const gatedColumns = statuses.map((col: any) => ({
      ...col,
      criteria: Array.isArray(resolvedGates[col.id]) && resolvedGates[col.id].length
        ? resolvedGates[col.id]
        : (Array.isArray(col.gate) ? col.gate : []),
    })).filter((col: any) => col.criteria.length > 0);
    const move = (data: object) => request.patch(`/api/board/${card}`, { headers: workerAuth, data });
    const statusOf = async () => (await (await request.get(`/api/board/${card}`, { headers: auth })).json()).status;

    const reassign = await move({ session: `${worker}-peer` });
    expect(reassign.status(), 'a worker must not move its existing card onto another worker\'s board').toBe(403);
    expect((await reassign.json()).code).toBe('cross_board_reassignment_forbidden');

    for (const col of gatedColumns) {
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
      if (col.criteria.length > 1) {
        const partial = await move({ status: col.id, gate_checked: [col.criteria[0]] });
        expect(partial.status(), `${col.id} must refuse a partial acknowledgement`).toBe(409);
      }
      expect(await statusOf(), `${col.id} must still not have moved`).not.toBe(col.id);
    }

    // 3. The honest path works. `review` is the check: `done` additionally
    //    demands an asset link and `verified` is owner-gated, so neither can
    //    stand for "a complete acknowledgement is accepted".
    const review = gatedColumns.find((s: any) => s.id === 'review');
    const reviewer = `${worker}-peer`;
    const ack = await move({ status: 'review', reviewer, gate_checked: review.criteria });
    const ackBody = await ack.json();
    expect(ack.ok(), `a complete, exact acknowledgement must be accepted: ${JSON.stringify(ackBody)}`).toBeTruthy();
    expect(await statusOf()).toBe('review');
    expect((await (await request.get(`/api/board/${card}`, { headers: auth })).json()).reviewer,
      'the owner may link a peer as reviewer without transferring board ownership').toBe(reviewer);

    // `done` refuses even a complete gate ack with no artifact to point at.
    const done = gatedColumns.find((s: any) => s.id === 'done');
    const noAsset = await move({ status: 'done', gate_checked: done.criteria });
    expect(noAsset.status(), 'done must refuse a card that names no artifact').toBe(409);
    expect((await noAsset.json()).code).toBe('done_requires_asset_link');

    const artifact = 'e2e/worker-lifecycle.spec.ts';
    const withArtifact = await move({ desc_append: `\nArtifact: ${artifact}` });
    expect(withArtifact.ok(), 'the actual lifecycle spec must be linkable as the produced artifact').toBeTruthy();
    const noEvidence = await move({ status: 'done', gate_checked: done.criteria });
    expect(noEvidence.status(), 'an artifact link alone is still a plan, not evidence that work ran').toBe(409);
    expect((await noEvidence.json()).code).toBe('done_requires_evidence');
    const evidence = `ran \`npx playwright test -c e2e/playwright.config.ts ${artifact} --project=${testInfo.project.name}\`; browser exercised creation, ${modelFamily} selection, prompt persistence, stable terminal geometry, ownership refusals, and review gates`;
    const doneAck = await move({ status: 'done', gate_checked: done.criteria, evidence });
    const doneBody = await doneAck.json();
    expect(doneAck.ok(), `done must accept the exact gate once real artifact and execution evidence are linked: ${JSON.stringify(doneBody)}`).toBeTruthy();
    expect(await statusOf()).toBe('done');

    const verified = gatedColumns.find((s: any) => s.id === 'verified');
    const blanket = await move({ status: 'verified', gate_ack: true });
    expect(blanket.status(), 'verified must reject the blanket gate-ack backdoor').toBe(409);
    expect((await blanket.json()).code).toBe('verified_requires_gate_checked');
    const verifiedAck = await move({ status: 'verified', gate_checked: verified.criteria });
    expect(verifiedAck.ok(), 'verified must accept every exact criterion after review and done evidence').toBeTruthy();
    expect(await statusOf()).toBe('verified');

    // ── DELETE, and no operation strands in the outbox ────────────────────
    const del = await request.post(`/api/sessions/${worker}/delete`, { headers: humanAuth });
    const delBody = await del.json();
    expect(del.ok(), `the dashboard's guarded delete must be accepted: ${JSON.stringify(delBody)}`).toBeTruthy();
    await expect.poll(async () => (await request.get(`/api/sessions/${worker}`, { headers: auth })).status(),
      { timeout: 30_000 }).toBe(404);

    await page.reload();
    await page.waitForFunction(() => typeof (window as any).fetchSessions === 'function');
    await expect(page.locator('text=/Unsaved changes/')).toHaveCount(0);
  } finally {
    if (card) await request.delete(`/api/board/${card}`, { headers: auth }).catch(() => {});
    await request.post(`/api/sessions/${worker}/delete`, { headers: humanAuth }).catch(() => {});
  }
});
}
