// Cross-worker work is a board lifecycle, not an ephemeral chat message. This
// drives the real Rust API and then verifies the real card UI: verified
// requester, source Messages link, durable terminal callback, and idempotency.
import { test, expect } from './fixtures';

test('worker request stays on one card and returns one terminal callback', async ({ page, request }, testInfo) => {
  await page.goto('/');
  const token = await page.evaluate(() => (window as any)._AMUX_AUTH_TOKEN as string);
  expect(token, 'served bootstrap must provide the API token').toBeTruthy();
  const auth = { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' };
  const suffix = `${testInfo.project.name}-${Date.now()}`;
  const requester = `callback-a-${suffix}`;
  const worker = `callback-b-${suffix}`;
  let card = '';
  let parent = '';
  let independent = '';

  try {
    // Different groups are deliberate: fresh installs are open for peer
    // discovery/message delivery, while an explicit worker opt-out remains a
    // supported configuration. No approval banner should be involved.
    for (const [name, group] of [[requester, 'requesters'], [worker, 'workers']]) {
      const made = await request.post('/api/sessions', {
        headers: auth,
        data: { name, dir: '/tmp', tags: [`e2e-${group}`] },
      });
      expect(made.status(), `create ${name}`).toBe(201);
    }

    const parentMade = await request.post('/api/board', {
      headers: { ...auth, 'X-Amux-Worker': requester },
      data: {
        title: 'Assemble the callback acceptance result',
        desc: 'Use the delegated artifact, then finish the requester workflow.',
        status: 'doing', session: requester, type: 'chore',
      },
    });
    expect(parentMade.status()).toBe(201);
    parent = (await parentMade.json()).id;
    const independentMade = await request.post('/api/board', {
      headers: { ...auth, 'X-Amux-Worker': requester },
      data: {
        title: 'Independent callback acceptance work',
        desc: 'This remains ready while the delegated result is open.',
        status: 'todo', session: requester, type: 'chore',
      },
    });
    expect(independentMade.status()).toBe(201);
    independent = (await independentMade.json()).id;
    for (const [id, next_action] of [
      [parent, 'Integrate the returned dependency into the requester result.'],
      [independent, 'Run the independent callback verification.'],
    ]) {
      const progressed = await request.patch(`/api/board/${encodeURIComponent(id)}`, {
        headers: { ...auth, 'X-Amux-Worker': requester }, data: { next_action },
      });
      expect(progressed.ok(), `record continuation for ${id}`).toBeTruthy();
    }

    const made = await request.post('/api/board', {
      headers: { ...auth, 'X-Amux-Worker': requester },
      data: {
        title: 'Produce the callback acceptance result',
        desc: 'Create /tmp/amux-callback-e2e/result.md and record it on this card.',
        status: 'todo',
        session: worker,
        type: 'chore',
        request_parent: parent,
        callback: { prompt: 'Continue the requester workflow from the recorded result.' },
      },
    });
    expect(made.status()).toBe(201);
    const initial = await made.json();
    card = initial.id;
    expect(initial.requested_by).toBe(requester);
    expect(initial.callback).toMatchObject({ session: requester, state: 'armed' });
    expect(initial.request_dependency).toEqual({
      verdict: 'parent_requeued', linked: true, parent, child: card,
      parent_status: 'todo', prior_parent_status: 'doing',
    });

    const parentWaiting = await request.get(`/api/board/${encodeURIComponent(parent)}`, { headers: auth });
    expect(parentWaiting.ok()).toBeTruthy();
    expect(await parentWaiting.json()).toMatchObject({ status: 'todo', depends_on: [card] });
    const waitingFrontier = await request.get(`/api/board/ready?session=${encodeURIComponent(requester)}`, {
      headers: auth,
    });
    expect(waitingFrontier.ok()).toBeTruthy();
    const waitingState = await waitingFrontier.json();
    expect(waitingState.wip).toMatchObject({ doing: 0, holding: [] });
    expect(waitingState.ready.map((row: any) => row.id)).toContain(independent);
    expect(waitingState.ready.map((row: any) => row.id)).not.toContain(parent);

    const finished = await request.patch(`/api/board/${encodeURIComponent(card)}`, {
      headers: { ...auth, 'X-Amux-Worker': worker },
      data: {
        status: 'done',
        force: true,
        reason: 'The callback transport is the subject; unrelated task-type gates are outside this fixture.',
        evidence: '/tmp/amux-callback-e2e/result.md',
        last_result: 'Wrote /tmp/amux-callback-e2e/result.md and validated the callback path.',
      },
    });
    expect(finished.ok()).toBeTruthy();
    const terminal = await finished.json();
    expect(terminal.callback_dispatch).toEqual({ attempted: 1, queued: 1, refused: 0 });
    expect(terminal.callback).toMatchObject({
      session: requester,
      state: 'queued',
      message_id: `task-callback-${card}`,
    });

    const resumedFrontier = await request.get(`/api/board/ready?session=${encodeURIComponent(requester)}`, {
      headers: auth,
    });
    expect(resumedFrontier.ok()).toBeTruthy();
    expect((await resumedFrontier.json()).ready.map((row: any) => row.id)).toContain(parent);

    // Retrying the terminal update is a chaos/replay cell. The stable outbox id
    // must keep both the steering queue and Messages ledger at one callback.
    const replay = await request.patch(`/api/board/${encodeURIComponent(card)}`, {
      headers: { ...auth, 'X-Amux-Worker': worker },
      data: {
        status: 'done', force: true,
        reason: 'Replay the already-terminal mutation to verify callback idempotency.',
      },
    });
    expect(replay.ok()).toBeTruthy();

    const history = await request.get(`/api/history?q=${encodeURIComponent(card)}&limit=20`, {
      headers: auth,
    });
    expect(history.ok()).toBeTruthy();
    const linked = (await history.json()).filter((m: any) => m.card_id === card);
    expect(linked.filter((m: any) => m.delivery === 'board')).toHaveLength(1);
    expect(linked.filter((m: any) => m.type === 'task-callback')).toHaveLength(1);

    // The compact card UI must expose the same facts and clickable source link;
    // this is the operator-facing acceptance, not only an API assertion.
    await page.goto(`/#issue=${encodeURIComponent(card)}`);
    await expect(page.locator('#board-detail-overlay')).toHaveClass(/active/, { timeout: 30_000 });
    const meta = page.locator('#bd-meta');
    await expect(meta).toContainText('Source message');
    await expect(meta).toContainText('Worker request');
    await expect(meta).toContainText(requester);
    await expect(meta).toContainText('queued');
    await expect(meta).toContainText('/tmp/amux-callback-e2e/result.md');
    await expect(meta.locator('button', { hasText: /^MSG-/ })).toHaveCount(2);
  } finally {
    if (card) await request.delete(`/api/board/${encodeURIComponent(card)}`, { headers: auth }).catch(() => {});
    if (parent) await request.delete(`/api/board/${encodeURIComponent(parent)}`, { headers: auth }).catch(() => {});
    if (independent) await request.delete(`/api/board/${encodeURIComponent(independent)}`, { headers: auth }).catch(() => {});
    await request.delete(`/api/sessions/${requester}`, { headers: auth }).catch(() => {});
    await request.delete(`/api/sessions/${worker}`, { headers: auth }).catch(() => {});
  }
});

test('a worker cannot redirect its callback to a third worker', async ({ page, request }, testInfo) => {
  await page.goto('/');
  const token = await page.evaluate(() => (window as any)._AMUX_AUTH_TOKEN as string);
  const auth = { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' };
  const suffix = `${testInfo.project.name}-${Date.now()}`;
  const requester = `callback-owner-${suffix}`;
  const worker = `callback-target-${suffix}`;
  const third = `callback-third-${suffix}`;
  let validCard = '';

  try {
    for (const name of [requester, worker, third]) {
      expect((await request.post('/api/sessions', {
        headers: auth, data: { name, dir: '/tmp', tags: ['e2e-callback-security'] },
      })).status()).toBe(201);
    }
    const refused = await request.post('/api/board', {
      headers: { ...auth, 'X-Amux-Worker': requester },
      data: {
        title: 'Attempt callback redirect', status: 'todo', session: worker,
        callback: { session: third },
      },
    });
    expect(refused.status()).toBe(403);
    expect((await refused.json()).error).toContain('verified requester');

    const valid = await request.post('/api/board', {
      headers: { ...auth, 'X-Amux-Worker': requester },
      data: { title: 'Keep requester-owned callback', status: 'todo', session: worker, callback: true },
    });
    expect(valid.status()).toBe(201);
    validCard = (await valid.json()).id;
    const hijack = await request.patch(`/api/board/${encodeURIComponent(validCard)}`, {
      headers: { ...auth, 'X-Amux-Worker': worker },
      data: { callback: false },
    });
    expect(hijack.status()).toBe(403);
    expect((await hijack.json()).error).toContain('only the verified requester');
  } finally {
    if (validCard) await request.delete(`/api/board/${encodeURIComponent(validCard)}`, { headers: auth }).catch(() => {});
    for (const name of [requester, worker, third]) {
      await request.delete(`/api/sessions/${name}`, { headers: auth }).catch(() => {});
    }
  }
});
