import { test, expect } from '@playwright/test';

// Exercise the shipped renderer and actual header buttons, including ANSI spans
// that used to cross block boundaries and Codex's different prompt glyph.
test.beforeEach(async ({ page }) => {
  await page.route('**/api/sessions/nav-probe/peek?*', route => route.fulfill({ json: { output: '' } }));
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
    const raw = '\x1b[34m› Review the first paragraph and verify the implementation\n  with a second line.\n\n  Keep this paragraph together.\n❯ [amux-origin:backend] inspect the build\n› standalone request\nAssistant reply\n❯ \n❯ 1. Yes';
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
  const beacons: any[] = [];
  await page.route('**/api/client-debug', async route => {
    beacons.push(route.request().postDataJSON());
    await route.fulfill({ json: { ok: true } });
  });
  await page.evaluate(() => {
    eval('lastPeekHTML = "needle\\n" + "output\\n".repeat(60) + "needle"; peekSearchQuery = "needle";');
    (window as any).applyPeekSearch();
  });
  await page.getByRole('button', { name: 'Next message', exact: true }).click();
  await expect(page.locator('#peek-msg-count')).toHaveText('Matches 2/2');
  await expect.poll(() => beacons.filter(b => b.verdict === 'landed').length).toBe(1);
  await page.evaluate(() => {
    eval('peekSearchQuery = ""; _peekMsgNavKind = "human";');
    document.getElementById('peek-body')!.innerHTML = '';
  });
  await page.getByRole('button', { name: 'Next message', exact: true }).evaluate((el: HTMLElement) => el.click());
  await expect.poll(() => beacons.filter(b => b.verdict === 'no-targets').length).toBe(1);
  await expect(page.locator('#peek-msg-count')).toHaveText('Human 0');
});
