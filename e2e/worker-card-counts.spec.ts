import { test, expect, Page } from '@playwright/test';

// Ethan, 2026-08-11: "on the sched row in the card view of worker list page
// homepage put # of board items (total)".
//
// Seeds its own worker + cards: the e2e server starts with an empty board, and
// a count feature tested against zero data passes without rendering anything.

async function appToken(page: Page): Promise<string> {
  const tok = await page.evaluate(() => (window as any)._AMUX_AUTH_TOKEN as string);
  expect(tok, 'served bootstrap must inject a non-empty auth token').toBeTruthy();
  return tok;
}

test('worker card shows total board items on the sched row', async ({ page, request }) => {
  await page.goto('/');
  await page.waitForLoadState('networkidle');
  const token = await appToken(page);
  const auth = { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' };

  // A worker IS its env file, so create one the fleet list will show.
  const worker = 'e2e-count-lane';
  await request.post('/api/sessions', {
    headers: auth,
    data: { name: worker, dir: '/tmp', desc: 'e2e count fixture' },
  });

  // 3 items for this worker: 1 doing, 2 not. Total must read 3, doing 1.
  for (const st of ['doing', 'todo', 'backlog']) {
    const res = await request.post('/api/board', {
      headers: auth,
      data: { title: `e2e count ${st}`, status: st, session: worker, type: 'chore' },
    });
    expect(res.ok(), `seeding a ${st} card must succeed`).toBeTruthy();
  }

  await page.reload();
  await page.waitForLoadState('networkidle');

  const card = page.locator(`.session-card:has-text("${worker}"), [data-session="${worker}"]`).first();
  await expect(card, 'the seeded worker must appear in the card view').toBeVisible({ timeout: 15000 });

  const meta = card.locator('.meta-count');
  await expect(meta).toBeVisible();
  const text = (await meta.textContent()) || '';

  // The ASSERTION IS THE NUMBER, not merely that a total is present: a counter
  // that renders the wrong figure is worse than none, and "contains items"
  // would pass on any value.
  expect(text).toContain('3 items');
  expect(text).toContain('1 doing');
  expect(await card.locator('.mc-total').textContent()).toBe('3');

  // total >= doing must hold by construction (shared predicate).
  const tot = Number(await card.locator('.mc-total').textContent());
  const doing = Number(await card.locator('.mc-doing').textContent());
  expect(tot).toBeGreaterThanOrEqual(doing);

  // cleanup
  await request.delete(`/api/sessions/${worker}`, { headers: auth });
});
