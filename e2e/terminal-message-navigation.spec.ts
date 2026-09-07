import { test, expect } from './fixtures';

// Exercise the shipped renderer and actual header buttons, including ANSI spans
// that used to cross block boundaries and Codex's different prompt glyph.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('amux_walkthrough_done', '1'));
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as any).highlightPrompts === 'function');
  await page.evaluate(() => {
    document.querySelector('.wt-overlay')?.remove();
    eval("peekSession = 'nav-probe'; _peekMsgRowsFor = peekSession; _peekMsgRows = []; _peekMsgNavKind = 'all'; _peekMsgIndex = -1;");
    document.getElementById('peek-overlay')!.classList.add('active');
    (window as any)._stopPeekPoll();
  });
});

test('separate Codex/Claude blocks, multiline origin matching, and late scoped history', async ({ page }) => {
  const result = await page.evaluate(() => {
    const w = window as any;
    const body = document.getElementById('peek-body')!;
    const raw = '\x1b[34m› Review the first paragraph and verify the implementation\n  with a second line.\n\n  Keep this paragraph together.\n❯ [amux-origin:backend] inspect the build\n› standalone request\nAssistant reply\n❯ \n❯ 1. Yes\n› Ask Codex to do anything\n\n  gpt-6-astra xhigh · ~/Dev/amux';
    body.innerHTML = w._peekHtml(raw);
    const before = [...body.querySelectorAll('.peek-prompt')].map(el => (el as HTMLElement).dataset.msgKind);
    eval("_peekMsgRows = [{session:'nav-probe',type:'direct',text:'Review the first paragraph and verify the implementation with a second line. Keep this paragraph together.'}, {session:'other-worker',type:'direct',text:'standalone request'}]");
    w._peekReclassifyPrompts();
    return { before, after: [...body.querySelectorAll('.peek-prompt')].map(el => (el as HTMLElement).dataset.msgKind),
      nested: body.querySelectorAll('.peek-prompt .peek-prompt').length,
      first: body.querySelector('.peek-prompt')!.textContent,
      leakedReply: body.querySelector('.peek-prompt:last-of-type')?.textContent?.includes('Assistant reply'),
      colors: [...body.querySelectorAll('.peek-prompt')].map(el => getComputedStyle(el).borderLeftColor) };
  });
  expect(result.before).toEqual(['unknown', 'session', 'unknown']);
  expect(result.after).toEqual(['human', 'session', 'unknown']);
  expect(result.nested).toBe(0);
  expect(result.first).toContain('Keep this paragraph together.');
  expect(result.leakedReply).toBeFalsy();
  expect(new Set(result.colors).size).toBe(3);
});

test('header arrows land at the start of a long message and hold through refresh', async ({ page }) => {
  await page.evaluate(() => {
    const w = window as any;
    const raw = 'intro\n'.repeat(35) + '› first human message\n  ' + 'long paragraph '.repeat(500)
      + '\nAssistant\n' + 'output\n'.repeat(35) + '❯ [Scheduled] run checks\nAssistant\n' + 'tail\n'.repeat(40);
    eval('lastPeekHTML = _peekHtml(' + JSON.stringify(raw) + '); _lastLiveHTML = lastPeekHTML; _peekHistoryHTML = "";');
    w.applyPeekSearch(false, false);
    document.getElementById('peek-body')!.scrollTop = 0;
  });
  await page.getByRole('button', { name: 'Next message', exact: true }).click();
  const landing = await page.evaluate(() => {
    const body = document.getElementById('peek-body')!;
    const target = body.querySelector('.peek-msg-current')!;
    return { offset: target.getBoundingClientRect().top - body.getBoundingClientRect().top,
      top: body.scrollTop, locked: eval('_peekScrollLocked'), height: target.getBoundingClientRect().height };
  });
  expect(landing.height).toBeGreaterThan(400);
  expect(landing.offset).toBeGreaterThanOrEqual(0);
  expect(landing.offset).toBeLessThan(18);
  expect(landing.top).toBeGreaterThan(100);
  expect(landing.locked).toBe(true);
  await page.evaluate(() => (window as any)._peekReclassifyPrompts());
  expect(await page.locator('#peek-body').evaluate(el => el.scrollTop)).toBe(landing.top);
  await page.getByRole('button', { name: 'Next message', exact: true }).click();
  await expect(page.locator('.peek-msg-current')).toContainText('[Scheduled]');
  await page.getByRole('button', { name: 'Previous message', exact: true }).click();
  await expect(page.locator('.peek-msg-current')).toContainText('first human message');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: test.info().outputPath('terminal-navigation.png') });
});

test('search navigation shares real matches and reports an empty filter', async ({ page }) => {
  await expect(page.locator('#peek-search-wrap')).toBeHidden();
  const beacons: any[] = [];
  await page.route('**/api/client-debug', async route => {
    beacons.push(route.request().postDataJSON());
    await route.fulfill({ json: { ok: true } });
  });
  await page.evaluate(() => {
    eval('lastPeekHTML = "needle\\n" + "output\\n".repeat(60) + "needle";');
  });
  await page.getByRole('button', { name: 'Find in terminal', exact: true }).click();
  await page.getByRole('searchbox', { name: 'Find in terminal', exact: true }).fill('needle');
  await expect(page.locator('#peek-msg-kind')).toHaveValue('matches');
  await expect(page.locator('#peek-msg-kind')).toBeDisabled();
  await page.getByRole('button', { name: 'Next message', exact: true }).click();
  await expect(page.locator('#peek-msg-count')).toHaveText('2/2');
  await expect.poll(() => beacons.filter(b => b.verdict === 'landed').length).toBe(1);
  await page.locator('#peek-search').press('Enter');
  await expect(page.locator('#peek-msg-count')).toHaveText('1/2');
  await page.getByRole('button', { name: 'Next message', exact: true }).click();
  await expect(page.locator('#peek-msg-count')).toHaveText('2/2');
  await page.locator('#peek-search').press('Escape');
  await expect(page.locator('#peek-search-wrap')).toBeHidden();
  await expect(page.locator('#peek-overlay')).toBeVisible();
  await expect(page.locator('#peek-msg-kind')).toBeEnabled();
  await page.locator('#peek-msg-kind').selectOption('human');
  await page.evaluate(() => {
    document.getElementById('peek-body')!.innerHTML = '';
  });
  await page.route('**/api/sessions/nav-probe/log?*', route => route.fulfill({ status: 404, json: { error: 'missing' } }));
  await page.getByRole('button', { name: 'Next message', exact: true }).click();
  await expect.poll(() => beacons.filter(b => b.verdict === 'no-targets').length).toBe(1);
  await expect(page.locator('#peek-msg-count')).toHaveText('0');
  await expect(page.locator('#toast')).toContainText('This worker has no saved earlier output.');
});

test('toolbar has one horizontal row, explicit filters and reachable named actions', async ({ page }) => {
  await page.evaluate(() => {
    eval("sessions.push({name:'nav-probe',dir:'/tmp/toolbar-probe',running:true}); peekSessionDir='/tmp/toolbar-probe';");
    (window as any)._peekMsgCount([]);
  });
  const geometry = await page.locator('.peek-toolbar').evaluate(el => ({
    height: el.getBoundingClientRect().height,
    overflow: el.scrollWidth > el.clientWidth + 1,
    controls: [...el.querySelectorAll('button,select')].map(c => {
      const r = c.getBoundingClientRect();
      return { width: r.width, height: r.height, visible: c.contains(document.elementFromPoint(r.x + r.width/2, r.y + r.height/2)) };
    }),
  }));
  expect(geometry.height).toBeLessThanOrEqual(48);
  expect(geometry.overflow).toBe(false);
  for (const c of geometry.controls) {
    expect(c.width).toBeGreaterThanOrEqual(44);
    expect(c.height).toBeGreaterThanOrEqual(44);
    expect(c.visible).toBe(true);
  }
  await page.getByRole('combobox', { name: 'Message type' }).selectOption('session');
  expect(await page.evaluate(() => eval('_peekMsgNavKind'))).toBe('session');
  await expect(page.locator('#peek-msg-count')).toHaveText('0');
  // Empty loaded output may still have earlier messages; these remain actions.
  await expect(page.getByRole('button', { name: 'Previous message', exact: true })).not.toHaveAttribute('aria-disabled', 'true');
  await page.locator('#peek-overlay').getByRole('button', { name: 'Worker actions', exact: true }).click();
  await expect(page.locator('#peek-more-dropdown [data-worker-action="directory"]')).toHaveText('📁Change directory');
  await expect(page.locator('#peek-more-dropdown [data-worker-action="copy-directory-link"]')).toHaveText('🔗Copy directory link');
  await expect(page.locator('.peek-dir-bar .card-dir-edit')).toHaveCount(0);
  await page.locator('#peek-overlay').getByRole('button', { name: 'Worker actions', exact: true }).click();
  const tabs = page.getByRole('button', { name: 'Customize worker tabs' });
  await expect(tabs).toContainText('Tabs');
  const box = await tabs.boundingBox();
  expect(box!.width).toBeGreaterThanOrEqual(44);
  expect(box!.height).toBeGreaterThanOrEqual(44);
  expect(box!.x + box!.width).toBeLessThanOrEqual(page.viewportSize()!.width);
  await tabs.click();
  await expect(page.locator('#peek-tab-customizer-menu')).toBeVisible();
  await expect(tabs).toHaveAttribute('aria-expanded', 'true');
});

test('a toolbar layout regression announces itself to client diagnostics', async ({ page }) => {
  const beacons: any[] = [];
  await page.route('**/api/client-debug', async route => {
    beacons.push(route.request().postDataJSON());
    await route.fulfill({ json: { ok: true } });
  });
  await page.locator('#peek-msg-nav').evaluate(el => { (el as HTMLElement).style.flexDirection = 'column'; });
  await page.evaluate(() => (window as any)._peekToolbarCheck());
  await expect.poll(() => beacons.filter(b => b.kind === 'peek-toolbar-layout').length).toBe(1);
  const signal = beacons.find(b => b.kind === 'peek-toolbar-layout');
  expect(signal.verdict).toBe('unusable-controls');
  expect(signal.measured).toBe(true);
  expect(signal.n_considered).toBeGreaterThan(0);
});

test('a scroll gesture ending over a message arrow is inert', async ({ page }) => {
  const beacons: any[] = [];
  await page.route('**/api/client-debug', async route => {
    beacons.push(route.request().postDataJSON());
    await route.fulfill({ json: { ok: true } });
  });
  await page.evaluate(() => {
    const body = document.getElementById('peek-body')!;
    body.innerHTML = '<div style="height:4000px">plain output without prompts</div>';
    body.scrollTop = 0;
    document.getElementById('toast')!.classList.remove('visible');
  });
  const next = page.getByRole('button', { name: 'Next message', exact: true });
  const box = await next.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
  await page.mouse.down();
  await page.evaluate(() => {
    const body = document.getElementById('peek-body')!;
    body.scrollTop = 500;
    body.dispatchEvent(new Event('scroll'));
  });
  await page.mouse.up();

  await expect.poll(() => beacons.filter(b => b.verdict === 'suppressed-scroll-gesture').length).toBe(1);
  await expect(page.locator('#toast')).not.toHaveClass(/visible/);
  expect(await page.locator('#peek-body').evaluate(el => el.scrollTop)).toBe(500);
});

test('an explicit empty navigation loads earlier output and lands on its message', async ({ page }) => {
  const beacons: any[] = [];
  await page.route('**/api/client-debug', async route => {
    beacons.push(route.request().postDataJSON());
    await route.fulfill({ json: { ok: true } });
  });
  await page.route('**/api/sessions/nav-probe/log?*', route => route.fulfill({
    status: 200,
    headers: { 'Content-Type': 'text/plain', 'X-Log-Remaining': '0' },
    body: '› older human request\nassistant response\n',
  }));
  await page.evaluate(() => {
    eval('peekSearchQuery = ""; _peekMsgNavKind = "all"; _peekEarlier = { chunks: [], loadedKb: 0, done: false, hidden: false, loading: false }; _peekHistoryHTML = ""; _lastLiveHTML = ""; lastPeekHTML = "";');
    document.getElementById('peek-body')!.innerHTML = '';
    document.getElementById('toast')!.classList.remove('visible');
  });
  await page.getByRole('button', { name: 'Next message', exact: true }).click();

  await expect(page.locator('.peek-msg-current')).toContainText('older human request');
  await expect.poll(() => beacons.filter(b => b.verdict === 'loaded-earlier').length).toBe(1);
  await expect.poll(() => beacons.filter(b => b.verdict === 'landed').length).toBe(1);
  await expect(page.locator('#toast')).not.toHaveClass(/visible/);
});
