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
  // enterFiles() below calls boot() three times on the SAME page — each one a
  // fresh page.goto('/'), but NOT a fresh origin, so localStorage survives
  // across them. App boot restores the persisted view (_restoreScreen reads
  // `amux_ui_view`) BEFORE any of this file's own seeding/clicking runs, at
  // _filesPath's freshly-reset default of '/' — so the SECOND and THIRD calls
  // in a test can auto-navigate into Files at '/' first, then race the click
  // handler's own correct navigation to '/tmp/'. Whichever response lands
  // last wins, which is why this showed up as an inconsistent path (not the
  // consistent "never navigated" shape the seeding bug produced). An
  // addInitScript clears the one key that persists this, before EVERY
  // navigation on this page, so each boot() is really starting fresh.
  await page.addInitScript(() => { try { localStorage.removeItem('amux_ui_view'); } catch (e) {} });
  // Third AMUX-122 attempt. The first two (seed via page.evaluate's string
  // form; clear the persisted view) were each real, necessary fixes — and
  // still not sufficient, because they both poke `sessions` once and hope
  // nothing overwrites it before the click. Something does: the app's own
  // fetchSessions() poll runs on load AND periodically, and across THREE
  // sequential enterFiles() calls in one test (real page loads, real
  // navigation, real assertion retries) enough wall-clock time elapses that
  // a live poll can land between this seed and the click, silently replacing
  // the one-shot `sessions = [SAMPLE]` with whatever the real dev/CI server
  // actually has running — which does not include a worker named
  // 'ate-44-worker', so `_browseWorkerFiles` computed an empty root again on
  // the third call specifically (more elapsed time = more chances), and
  // toasted "This worker has no directory to browse" exactly like the very
  // first (pre-any-fix) failure.
  //
  // The robust fix, and the one this whole suite already uses everywhere
  // else (see fixtures.ts's entire reason for existing): mock the REQUEST,
  // not the client variable. Every fetchSessions() call — first load and
  // every later poll — now gets the fixture, so `sessions` stays correctly
  // seeded for the FULL test, immune to timing.
  //
  // enterFiles() calls boot() up to three times on the SAME page, so guard
  // against registering this route more than once per test: fixtures.ts's
  // own AF-47 wrapper fails a stub with zero hits at teardown, and a second
  // page.route() on an identical pattern shadows the first (last registered
  // wins interception) — the first two of three registrations would each
  // end up with zero hits and fail the test on their own safety net.
  const routed = page as unknown as { _sessionsRouted?: boolean };
  if (!routed._sessionsRouted) {
    routed._sessionsRouted = true;
    await page.route('**/api/sessions', (route) => route.fulfill({ json: [SAMPLE] }));
  }
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as any)._renderWorkerActionMenu === 'function');
  // Wait for the mocked response to actually have landed in `sessions`
  // before seeding the peek — a bare Array.isArray/some() check in STRING
  // form (see the note on page.evaluate below for why string, not function).
  await page.waitForFunction(`Array.isArray(sessions) && sessions.some(s => s.name === ${JSON.stringify(NAME)})`);
  // openPeek() is a genuine top-level FUNCTION DECLARATION, not a `let` —
  // the classic bundle attaches it to `window` automatically, so calling it
  // via a normal (function-form) page.evaluate correctly reaches its own
  // closure over peekSession/peekSessionDir regardless of how it's invoked.
  // That asymmetry (declarations reach window; `let`s do not) is the root
  // fact this whole file's earlier attempts kept working around instead of
  // using directly.
  await page.evaluate((name) => { (window as any).openPeek(name); }, NAME);
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
