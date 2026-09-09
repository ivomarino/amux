import { test, expect } from '@playwright/test';
test('live Amux browser creates durable work, records a note, then archives only its own test item', async ({
  page,
  request,
}) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  const core = process.env.AMUX_CORE_URL || 'https://localhost:8824';
  const title = `Business UI E2E — ${Date.now()}`;
  let id = '';
  try {
    await page.goto(process.env.AMUX_LIVE_PATH || '/');
    await expect(
      page.getByText('Live connection', { exact: true }),
    ).toBeVisible();
    await page
      .getByRole('button', { name: 'Create work', exact: true })
      .first()
      .click();
    await page.getByLabel('What needs to be done?').fill(title);
    await page
      .getByLabel('Instructions and context')
      .fill(
        'Isolated browser acceptance check. Leave in backlog; no external actions or agent execution.',
      );
    await page
      .getByRole('dialog')
      .getByRole('button', { name: 'Create work', exact: true })
      .click();
    await expect(
      page.getByRole('heading', { name: title, exact: true }),
    ).toBeVisible();
    const rows = await (
      await request.get(core + '/api/board?limit=2000')
    ).json();
    id = rows.find((t: any) => t.title === title)?.id || '';
    expect(id).not.toBe('');
    await page
      .getByLabel('Add a note')
      .fill('Browser-to-core persistence verified.');
    await page.getByRole('button', { name: 'Save review note' }).click();
    await expect(page.getByLabel('Add a note')).toHaveValue('');
    const item = await (await request.get(core + '/api/board/' + id)).json();
    expect(item.status).toBe('backlog');
    expect(item.desc).toContain('Browser-to-core persistence verified.');
    await page.keyboard.press('Escape');
    await page.reload();
    await expect(
      page.getByText('Live connection', { exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: 'test-results/live-business-desktop.png',
      fullPage: true,
    });
    expect(errors).toEqual([]);
  } finally {
    if (!id) {
      const r = await request.get(core + '/api/board?limit=2000');
      if (r.ok()) {
        const rows = await r.json();
        id = rows.find((t: any) => t.title === title)?.id || '';
      }
    }
    if (id) {
      const r = await request.post(core + '/api/board/' + id + '/archive', {
        data: {},
      });
      expect(r.ok()).toBe(true);
    }
  }
});
