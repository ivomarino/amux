import { test, expect } from './fixtures';

const NAME = 'ate-44-worker';
const ROOT = '/tmp/';

const SAMPLE = {
  name: NAME,
  dir: ROOT,
  desc: 'ATE-44 menu parity fixture',
  tags: ['e2e'],
  provider: 'claude',
  flags: '--model claude-sonnet-4 --effort high',
  active_model: 'claude-sonnet-4',
  task_override: '',
  pinned: false,
  yolo: false,
  isolated: false,
  auto_drain_backlog: true,
  spans_groups: true,
  spans_groups_value: '*',
  spans_groups_own: true,
  running: true,
  status: 'idle',
};

async function boot(page: import('@playwright/test').Page) {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as any)._renderWorkerActionMenu === 'function');
  // `sessions`/`peekSession`/`peekSessionDir` are top-level lexical (`let`)
  // bindings in the classic app bundle, not window properties — the comment
  // this replaces already knew that much, but drew the wrong conclusion from
  // it. A nested eval() INSIDE a page.evaluate(fn, arg) callback still runs in
  // that callback's own isolated scope (Playwright drives it via
  // Runtime.callFunctionOn): it cannot see or reassign another script's `let`
  // bindings, so this silently no-op'd every time. `sessions` stayed `[]`,
  // `_browseWorkerFiles` computed an empty root from it, and every click in
  // this file just toasted "This worker has no directory to browse" instead
  // of navigating — which is why #path= never appeared in the URL.
  //
  // page.evaluate's STRING form is different: Playwright sends it as a
  // top-level Runtime.evaluate, which — like typing consecutive lines into
  // the DevTools console — DOES interact with previously-declared top-level
  // `let`/`const` in the page's real execution context. Confirmed working
  // precedent: peek-default-tab.spec.ts's `sessions = [...]; openPeek(...)`.
  await page.evaluate(`
    sessions = ${JSON.stringify([SAMPLE])};
    peekSession = ${JSON.stringify(NAME)};
    peekSessionDir = ${JSON.stringify(ROOT)};
  `);
}

test('worker card and peek share all 25 worker actions, plus both peek-only actions', async ({ page }) => {
  await boot(page);
  const state = await page.evaluate((sample) => {
    const w = window as any;
    const card = document.createElement('div');
    card.innerHTML = w._renderWorkerActionMenu(sample, 'card');
    w._renderPeekWorkerActions(sample);
    const peek = document.getElementById('peek-more-dropdown')!;
    document.getElementById('peek-overlay')!.classList.add('active');
    peek.classList.add('open');
    const keys = (root: ParentNode) => Array.from(root.querySelectorAll('[data-worker-action]'))
      .map((el) => (el as HTMLElement).dataset.workerAction);
    const style = getComputedStyle(peek);
    return {
      card: keys(card),
      peek: keys(peek),
      peekOnly: Array.from(peek.querySelectorAll('[data-peek-action], #peek-focus-btn'))
        .map((el) => (el.textContent || '').trim()),
      overflowY: style.overflowY,
      maxHeight: style.maxHeight,
      scrollHeight: peek.scrollHeight,
      clientHeight: peek.clientHeight,
      headerIds: document.querySelectorAll('#peek-worker-menu-btn').length,
      composerIds: document.querySelectorAll('#peek-composer-more-btn').length,
      legacyDuplicateIds: document.querySelectorAll('#peek-more-btn').length,
    };
  }, SAMPLE);

  expect(state.card).toHaveLength(25);
  expect(state.peek).toEqual(state.card);
  // Every menu item in this codebase renders an icon span before its label
  // (see _renderWorkerActionMenu's own `<span class="mi">` + label pattern),
  // and .textContent naturally includes that child span's text — these are
  // real DOM icon glyphs, not a CSS ::before. Bare 'File browser'/'Focus mode'
  // was never what got rendered; it was a wrong expectation on a fresh test.
  expect(state.peekOnly).toEqual(['\u{1F4C2}File browser', '▴Focus mode']);
  expect(state.overflowY).toBe('auto');
  expect(state.maxHeight).not.toBe('none');
  expect(state.scrollHeight).toBeGreaterThan(state.clientHeight);
  await page.locator('#peek-focus-btn').scrollIntoViewIfNeeded();
  await expect(page.locator('#peek-focus-btn')).toBeVisible();
  expect(state.headerIds).toBe(1);
  expect(state.composerIds).toBe(1);
  expect(state.legacyDuplicateIds).toBe(0);
});

async function enterFiles(page: import('@playwright/test').Page, source: 'peek-file-browser' | 'peek-directory' | 'browse-files') {
  await boot(page);
  await page.evaluate(({ sample, source }) => {
    const w = window as any;
    w._renderPeekWorkerActions(sample);
    if (source === 'peek-directory') {
      document.getElementById('peek-dir-text')!.click();
    } else {
      document.querySelector<HTMLElement>(source === 'peek-file-browser'
        ? '[data-peek-action="file-browser"]' : '[data-worker-action="browse-files"]')!.click();
    }
  }, { sample: SAMPLE, source });
  await expect(page).toHaveURL(/#path=\/tmp\/$/);
  // Same top-level-`let` visibility problem as boot() above, on the read
  // side: `_exploreSession`/`_filesPath`/`activeView` are the classic
  // bundle's own lexical bindings, unreachable from a nested eval() inside a
  // page.evaluate(fn) callback. The string form runs as a top-level
  // Runtime.evaluate and can read them directly.
  const raw = await page.evaluate(`JSON.stringify({
    session: _exploreSession,
    root: _filesPath,
    activeView,
    filesVisible: getComputedStyle(document.getElementById('files-view')).display !== 'none',
    filesTabSelected: document.getElementById('tab-files').classList.contains('active'),
  })`);
  return JSON.parse(raw as string);
}

test('peek file entries produce the exact same canonical Files route state', async ({ page }) => {
  const fileBrowser = await enterFiles(page, 'peek-file-browser');
  const directoryPath = await enterFiles(page, 'peek-directory');
  const sharedBrowse = await enterFiles(page, 'browse-files');

  const expected = {
    session: NAME,
    root: ROOT,
    activeView: 'files',
    filesVisible: true,
    filesTabSelected: true,
  };
  expect(fileBrowser).toEqual(expected);
  expect(directoryPath).toEqual(fileBrowser);
  expect(sharedBrowse).toEqual(fileBrowser);
});
