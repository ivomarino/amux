import { test, expect } from '@playwright/test';

/**
 * The Lineage tab must reach the phone, and must not upgrade a weak answer
 * (AMUX-2393).
 *
 * `GET /api/why/task/<id>` is a careful instrument — it cites a table and row
 * for every line, reports every source it consulted INCLUDING those that
 * returned nothing with the predicate they ran, and answers `partial` or
 * `cannot_tell` rather than narrating. All of that care is destroyable by its
 * renderer, and a printer is exactly where an explainer reintroduces the
 * confident narration the API went to the trouble of avoiding.
 *
 * `scripts/test-lineage-render.sh` pins the render LOGIC against a real payload.
 * This file pins the two things that logic test structurally cannot see:
 *
 *  1. the panel is REACHABLE — the deep link opens the card ON the tab, and the
 *     fetch actually resolves in a browser
 *  2. it FITS at 375px with nothing overflowing
 *
 * (2) is the reason this exists as a browser test at all. `.claude/rules/
 * css-mobile.md` has said "test that flex containers don't overflow on 375px-wide
 * screens" for a long time with nothing enforcing it, and this panel is a genuine
 * candidate: it renders a fixed timestamp gutter, long SQL predicates and
 * untruncated gap prose, any of which will push past the viewport if the media
 * query is dropped. amux is mobile-first, so an overflow here is a real defect
 * rather than a cosmetic one.
 */

test.describe('board card lineage tab', () => {
  // NO `test.use({ viewport })` here. Each project in playwright.config.ts sets
  // its own (desktop 1280, mobile 375, ios-safari iPhone 15) against its own
  // server on its own port, so pinning one here would run this spec at a width
  // the project did not choose and quietly delete the ios-safari coverage that
  // config exists to provide. The mobile-only assertions below gate on the
  // measured width instead.

  test('deep-links to the tab, renders the verdict and gaps, and fits the viewport', async ({ page, request }) => {
    const errors: string[] = [];
    page.on('pageerror', e => errors.push(`pageerror: ${e.message}`));

    // Load first, only to lift the auth token the API requires. Same shape as
    // board-slim-consumers.spec.ts.
    await page.goto('/');
    const token = await page.evaluate(() => (window as never as { _AMUX_AUTH_TOKEN: string })._AMUX_AUTH_TOKEN);
    const auth = { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' };

    // CREATE the subject. Each project boots a server with a fresh temp home, so
    // the board is empty — a spec that picked an existing card would skip on
    // every CI run and look like coverage while asserting nothing.
    const created = await request.post('/api/board', {
      headers: auth,
      data: { title: 'lineage-tab e2e subject', status: 'todo', type: 'chore' },
    });
    expect(created.ok(), 'must be able to create a card to trace').toBeTruthy();
    const card = (await created.json()).id as string;
    expect(card, 'created card must have an id').toBeTruthy();

    await page.goto(`/#issue=${encodeURIComponent(card)}:lineage`);

    // The deep link is half the feature: a tab reachable only by tapping cannot
    // be linked, quoted in a nudge, or driven by the simulator rig.
    const tab = page.locator('#bd-tab-lineage');
    await expect(tab, 'the lineage tab must be active from the deep link alone').toHaveClass(/active/, { timeout: 30_000 });

    const panel = page.locator('#bd-lineage');
    await expect(panel).toBeVisible();
    // Wait for the fetch to land — asserting on the loading state would pass
    // against a panel that never resolves, which is the failure worth catching.
    await expect(panel, 'the payload must actually arrive, not sit on the loading line')
      .not.toContainText('Loading lineage', { timeout: 30_000 });

    // A verdict always renders. It leads the panel because a reader who scrolls
    // a plausible trail and never reaches the caveats has been misled by layout.
    await expect(panel.locator('.bd-lin-vlabel')).toHaveCount(1);

    // Whatever the endpoint reports must survive into the DOM. Compare against
    // the API rather than a fixture, so this fails if the renderer starts
    // dropping things the endpoint still sends.
    const why = await (await request.get(`/api/why/task/${encodeURIComponent(card)}`, { headers: auth })).json();
    const gaps: string[] = why.gaps ?? [];
    const zero = (why.sources ?? []).filter((s: { rows: number }) => !s.rows);

    // THE SPEC MUST NOT PASS VACUOUSLY. `toHaveCount(0)` against an empty
    // payload is zero compared with zero — it holds just as well against a
    // renderer that drops gaps entirely, which is the single failure this file
    // exists to catch. A freshly created card is thin by construction, so
    // confirm the payload can actually produce a positive before trusting that
    // it matched. If this ever fires, the assertions below are decoration and
    // the fix is a richer subject, not a deleted check.
    expect(
      gaps.length + zero.length,
      'the traced card produced neither gaps nor empty sources, so the two assertions below ' +
        'compare zero with zero and would pass against a renderer that drops both',
    ).toBeGreaterThan(0);

    await expect(
      panel.locator('.bd-lin-gaps li'),
      'EVERY gap must render — dropping one turns a hole into an apparently complete story',
    ).toHaveCount(gaps.length);

    if (zero.length) {
      await expect(
        panel.locator('.bd-lin-zero'),
        'a source that found NOTHING is evidence and must stay visible — hiding it recreates the ambiguity the endpoint reports predicates to avoid',
      ).toHaveCount(zero.length);
    }

    // AMUX-3607 gave part 3 a substrate for board transitions and only those.
    // The panel must state the BOUNDARY, not disappear: a reader seeing authz
    // lines on a card would otherwise assume scope writes and messages carry
    // one too, and their absence would read as unrestricted rather than
    // unrecorded.
    await expect(panel).toContainText('AMUX-3607');

    // THE LAYOUT ASSERTION, and the reason this is a browser test rather than
    // an extension of the render-logic script. Nothing inside the panel may
    // extend past the viewport — the panel renders a fixed timestamp gutter,
    // long SQL predicates and untruncated gap prose, each of which will push
    // past 375px if the media query is dropped.
    const width = page.viewportSize()!.width;
    const overflow = await page.evaluate(() => {
      const el = document.getElementById('bd-lineage');
      if (!el) return ['#bd-lineage missing'];
      return [...el.querySelectorAll('*')]
        .filter(n => n.getBoundingClientRect().right > window.innerWidth + 1)
        .slice(0, 5)
        .map(n => `${n.className} right=${Math.round(n.getBoundingClientRect().right)} > ${window.innerWidth}`);
    });
    expect(overflow, `nothing in the lineage panel may overflow a ${width}px viewport`).toEqual([]);

    // The tab is what a thumb hits; css-mobile.md requires 44px. Only asserted
    // where the mobile media query applies — demanding it at 1280px would be
    // asserting a rule the stylesheet does not claim, and the failure would
    // read as a layout bug rather than a wrong test.
    if (width <= 600) {
      const box = await tab.boundingBox();
      expect(box!.height, 'touch target must be at least 44px tall on mobile').toBeGreaterThanOrEqual(44);
    }

    expect(errors, 'the panel must not throw').toEqual([]);
  });
});
