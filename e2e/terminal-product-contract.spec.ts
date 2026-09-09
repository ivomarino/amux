import { type Route } from '@playwright/test';
import { test, expect, type Page, allowUnusedRoute } from './fixtures';

const worker = 'terminal-contract';

async function boot(page: Page, options?: {
  transcript?: string;
  live?: string;
  historyRows?: Record<string, unknown>[];
  liveDelayMs?: number;
  willSend?: boolean;
  waitForBothFrames?: boolean;
}) {
  let live = options?.live || 'Idle terminal\n';
  let peekResponses = 0;
  const transcript = options?.transcript || '';
  const historyRows = options?.historyRows || [];
  let persistedLayout: string | null = null;

  await page.addInitScript(() => localStorage.setItem('amux_walkthrough_done', '1'));
  await page.route(/\/api\/sessions(?:\?.*)?$/, route => route.fulfill({ json: [{
    name: worker, dir: '/tmp/terminal-contract', running: true, status: 'working',
  }] }));
  const tasksRoute = `**/api/sessions/${worker}/tasks`;
  await page.route(tasksRoute, route => route.fulfill({ json: { tasks: [], counts: {}, total: 0 } }));
  allowUnusedRoute(page, tasksRoute); // plan polling is throttled and optional to these terminal contracts
  await page.route(/\/api\/prefs(?:\?.*)?$/, async route => {
    if (route.request().method() === 'POST') {
      const body = route.request().postDataJSON() as { key?: string; value?: string };
      if (body.key === 'peek_tab_layout') persistedLayout = body.value || null;
      return route.fulfill({ json: { ok: true, key: body.key, value: body.value } });
    }
    const key = new URL(route.request().url()).searchParams.get('key') || '';
    return route.fulfill({ json: key === 'peek_tab_layout'
      ? { key, value: persistedLayout }
      : { key, value: null } });
  });
  await page.route(`**/api/history?limit=200&offset=0&session=${worker}`,
    route => route.fulfill({ json: historyRows }));
  const sendRoute = `**/api/sessions/${worker}/send`;
  await page.route(sendRoute, async (route: Route) => {
    const body = route.request().postDataJSON() as { text?: string };
    live = `\u276f ${body.text || ''}\nAssistant accepted the request\n`;
    await route.fulfill({ json: { ok: true, sent: true } });
  });
  if (!options?.willSend) allowUnusedRoute(page, sendRoute);
  await page.route(`**/api/sessions/${worker}/peek?*`, async route => {
    const liveOnly = new URL(route.request().url()).searchParams.has('live');
    if (liveOnly && options?.liveDelayMs) await new Promise(resolve => setTimeout(resolve, options.liveDelayMs));
    await route.fulfill({ json: liveOnly
      ? { name: worker, live, output: live, live_only: true }
      : { name: worker, history: transcript, live, output: live, output_is_viewport_only: true } });
    peekResponses += 1;
  });

  await page.goto('/');
  await page.waitForFunction(() => typeof (window as any).openPeek === 'function');
  await page.evaluate(name => {
    eval("_sendMode='send'");
    (window as any).openPeek(name);
  }, worker);
  await expect(page.locator('#peek-overlay')).toHaveClass(/active/);
  await expect(page.locator('#peek-body')).not.toContainText('Loading latest');
  // openPeek intentionally starts the fast live frame and the complete frame
  // concurrently. Wait for both so a slower engine cannot race a synthetic
  // frame against the still-arriving initial response.
  if (options?.waitForBothFrames !== false) {
    await expect.poll(() => peekResponses).toBeGreaterThanOrEqual(2);
  }
  return {
    setLive(value: string) { live = value; },
    getPersistedLayout() { return persistedLayout; },
  };
}

test('history/live seam renders every submitted prompt exactly once', async ({ page }) => {
  const oldOne = '[05:35 PM] first submitted request';
  const oldTwo = '[05:38 PM] second submitted request';
  const current = '[05:52 PM] newest submitted request';
  const transcript = `\u276f ${oldOne}\nAssistant first answer\n\u276f ${oldTwo}\nAssistant second answer\n`;
  const live = `\u276f ${oldOne}\nAssistant first answer\n\u276f ${oldTwo}\nAssistant second answer\n\u276f ${current}\nWorking now\n`;
  await boot(page, { transcript, live, historyRows: [
    { id: 1, session: worker, type: 'user', kind: 'human', text: oldOne, ts: Date.now() - 2 },
    { id: 2, session: worker, type: 'user', kind: 'human', text: oldTwo, ts: Date.now() - 1 },
    { id: 3, session: worker, type: 'user', kind: 'human', text: current, ts: Date.now() },
  ] });

  for (const text of [oldOne, oldTwo, current]) {
    await expect(page.locator('#peek-body .peek-prompt').filter({ hasText: text })).toHaveCount(1);
  }
  await expect(page.locator('#peek-body .peek-prompt-human')).toHaveCount(3);
});

test('a prompt sent while terminal is open is immediately attributed to the human', async ({ page }) => {
  await boot(page, { historyRows: [], willSend: true });
  const text = '[05:52 PM] sent from this open terminal';
  await page.locator('#peek-cmd-input').fill(text);
  await page.locator('.peek-cmd-bar .send-split-main').click();

  const prompt = page.locator('#peek-body .peek-prompt').filter({ hasText: text });
  await expect(prompt).toHaveCount(1);
  await expect(prompt).toHaveAttribute('data-msg-kind', 'human');
  await expect(prompt).toHaveAttribute('data-msg-label', 'Human');
  await expect(prompt).not.toContainText('Unclassified');
});

test('terminal controls are compact and new-output affordance appears only after buffering', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  const transcript = Array.from({ length: 220 }, (_, i) => `terminal output row ${i}`).join('\n');
  await boot(page, { transcript, live: 'latest output\n' });

  const layout = await page.evaluate(() => {
    const controls = document.querySelector('.peek-output-controls')!.getBoundingClientRect();
    const body = document.getElementById('peek-body')!.getBoundingClientRect();
    return { controlsWidth: controls.width, bodyWidth: body.width, controlsRight: controls.right, bodyRight: body.right };
  });
  expect(layout.controlsWidth).toBeLessThan(260);
  expect(Math.abs(layout.controlsRight - layout.bodyRight)).toBeLessThan(12);

  await page.evaluate(() => {
    const body = document.getElementById('peek-body')!;
    body.scrollTop = 0;
    body.dispatchEvent(new Event('scroll'));
  });
  await expect(page.locator('.scroll-lock-badge')).toBeHidden();

  await page.evaluate(async () => {
    const w = window as any;
    await w._queuePeekFrame(w._peekIdentity(), {
      name: 'terminal-contract', history: null, live: 'new buffered output\n',
    });
  });
  const notice = page.locator('.scroll-lock-badge');
  await expect(notice).toBeVisible();
  await expect(notice).toHaveText('New output \u2193');
  expect(await notice.evaluate(el => el.closest('#peek-body') !== null)).toBe(true);
  expect(await notice.evaluate(el => el.getBoundingClientRect().width)).toBeLessThan(180);
  await page.screenshot({ path: testInfo.outputPath('terminal-product-contract.png') });
});

test('terminal chrome cannot inject navigation or slash-picker keys', async ({ page }) => {
  const keyRequests: string[] = [];
  page.on('request', request => {
    if (/\/api\/sessions\/[^/]+\/(?:keys|send)/.test(request.url())) keyRequests.push(request.url());
  });
  await boot(page);

  const controls = page.locator('.peek-output-controls');
  await expect(controls.locator('button')).toHaveCount(1);
  await expect(controls.locator('[onclick*="peekQuickKeys"]')).toHaveCount(0);
  await controls.locator('#peek-copy-btn').click();
  await page.waitForTimeout(100);
  expect(keyRequests).toEqual([]);
  await expect(page.locator('#peek-cmd-input')).toHaveValue('');
});

test('worker tab choices restore from the server after browser storage is lost and the page reloads', async ({ page }) => {
  const state = await boot(page);
  await page.locator('#peek-tab-customize').click();
  const translate = page.locator('#peek-tab-customizer-menu .tab-customizer-item').filter({ hasText: 'Translate' });
  await expect(translate).toBeVisible();
  await translate.locator('input').uncheck();
  await expect(page.locator('#peek-tab-simple')).toBeHidden();
  await expect.poll(() => {
    const saved = state.getPersistedLayout();
    return saved ? JSON.parse(saved).hidden : [];
  }).toContain('simple');

  // Reproduce the failure mode more aggressively than a normal refresh: even
  // if browser storage has been purged, the server-backed layout must win.
  await page.evaluate(() => {
    localStorage.removeItem('amux_peek_hidden_tabs');
    localStorage.removeItem('amux_peek_tab_order');
  });
  await page.reload();
  await page.waitForFunction(() => typeof (window as any).openPeek === 'function');
  await page.evaluate(name => (window as any).openPeek(name), worker);
  await expect(page.locator('#peek-overlay')).toHaveClass(/active/);
  await expect(page.locator('#peek-tab-simple')).toBeHidden();
  await expect(page.locator('#peek-tab-steering')).toBeVisible();
});

test('terminal scroll geometry remains stable through repeated live frames', async ({ page }) => {
  const transcript = Array.from({ length: 900 }, (_, i) => `stable terminal row ${i}`).join('\n');
  await boot(page, { transcript, live: 'live frame zero\n' });
  const before = await page.evaluate(() => {
    const body = document.getElementById('peek-body')!;
    body.scrollTop = Math.floor((body.scrollHeight - body.clientHeight) * 0.45);
    const chunk = body.querySelector('.peek-render-chunk')!;
    return { top: body.scrollTop, contentVisibility: getComputedStyle(chunk).contentVisibility };
  });
  expect(before.contentVisibility).toBe('visible');

  await page.evaluate(async () => {
    const w = window as any;
    for (let i = 1; i <= 8; i++) {
      await w._queuePeekFrame(w._peekIdentity(), {
        name: 'terminal-contract', history: null, live: `live frame ${i}\n`,
      });
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    }
  });
  const after = await page.locator('#peek-body').evaluate(body => (body as HTMLElement).scrollTop);
  expect(Math.abs(after - before.top)).toBeLessThanOrEqual(1);
});

test('a delayed live-frame request cannot hold the terminal loading screen open', async ({ page }) => {
  const started = Date.now();
  await boot(page, {
    transcript: 'full response paints without waiting for live\n',
    live: 'current terminal frame\n',
    liveDelayMs: 3000,
    waitForBothFrames: false,
  });
  await expect(page.locator('#peek-body')).toContainText('current terminal frame', { timeout: 700 });
  expect(Date.now() - started).toBeLessThan(2500);
  await expect(page.locator('#peek-body .peek-loading')).toHaveCount(0);
});
