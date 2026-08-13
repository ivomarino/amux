import { test, expect } from '@playwright/test';

// Ethan, 2026-08-13: "I still can't view the tabs when I click the box which was
// historically a drop-down list of all the tabs." The ⊞ button (#peek-tab-customize)
// opens #peek-tab-customizer-menu — the show/hide/reorder list of every worker tab.
// A bottom-sheet fix shipped the same day and he still could not see it, so this
// asserts what a USER can see (geometry on screen), never that the element exists.
//
// The customizer is static markup, so it needs a visible overlay, not a live worker —
// openPeek() is called directly rather than seeding a session the harness has none of.
async function openPeek(page) {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as any).openPeek === 'function', { timeout: 20000 });
  await page.evaluate(() => (window as any).openPeek('e2e-probe'));
  await page.waitForSelector('#peek-overlay', { state: 'visible', timeout: 15000 });
}

test('the tab customizer opens and its rows are visible on screen', async ({ page }, info) => {
  await openPeek(page);
  const btn = page.locator('#peek-tab-customize');
  await expect(btn, 'the ⊞ button itself is missing').toBeVisible();
  await btn.click();

  const menu = page.locator('#peek-tab-customizer-menu');
  await expect(menu, 'menu did not become visible').toBeVisible();

  const vp = page.viewportSize()!;
  const box = await menu.boundingBox();
  expect(box, 'menu has no layout box').not.toBeNull();
  expect(box!.height, 'menu rendered with zero height').toBeGreaterThan(20);
  expect(box!.width, 'menu rendered with zero width').toBeGreaterThan(40);
  // On screen, not merely in the DOM — this is the actual complaint.
  expect(box!.y + box!.height, `menu is above the fold (y=${box!.y})`).toBeGreaterThan(0);
  expect(box!.y, `menu starts below the fold (y=${box!.y}, vh=${vp.height})`).toBeLessThan(vp.height);
  expect(box!.x + box!.width, 'menu is off-screen left').toBeGreaterThan(0);
  expect(box!.x, 'menu is off-screen right').toBeLessThan(vp.width);

  const items = menu.locator('.tab-customizer-item');
  expect(await items.count(), 'menu lists no tabs').toBeGreaterThan(3);
  const row = items.nth(1);
  await expect(row, 'a tab row is not visible').toBeVisible();
  const ib = await row.boundingBox();
  expect(ib!.height, 'a tab row has zero height').toBeGreaterThan(10);
  expect(ib!.y + ib!.height, 'first tab row is above the viewport').toBeGreaterThan(0);
  expect(ib!.y, 'first tab row is below the viewport').toBeLessThan(vp.height);

  await page.screenshot({ path: `../test-results/tabcust-${info.project.name}.png` });
});
