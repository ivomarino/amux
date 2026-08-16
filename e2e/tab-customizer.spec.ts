import { test, expect } from '@playwright/test';

// BLOCK THE SERVICE WORKER (AF-46). Nothing here tests offline behaviour, but
// sw.js reloads the page on `controllerchange` (app.js:24253) as soon as a
// freshly-registered worker claims the client — which, on the clean profile
// each project now gets, happens right where these tests call page.evaluate.
// The result was "Execution context was destroyed, most likely because of a
// navigation" on a spec about CSS geometry: a red that says nothing about the
// menu it is guarding. sw-fail-bar.spec.ts owns the worker's real behaviour.
test.use({ serviceWorkers: 'block' });

// The tab customizers moved OUT of the tab strips into Settings > Device >
// Appearance (they ate scarce tab-bar real estate; they are display
// preferences). The ⊞ triggers are now the "Main" (#tabs-customize) and
// "Worker" (#peek-tab-customize) buttons in that panel, and each opens its
// fixed-positioned .tab-customizer-menu. These tests assert what a USER can see
// (geometry on screen after opening the panel), never merely that an element
// exists — the original complaint was a menu that was in the DOM but off-screen.
async function openAppearance(page) {
  await page.goto('/');
  await page.waitForFunction(
    () => typeof (window as any).toggleSettings === 'function' && typeof (window as any)._settingsTab === 'function',
    { timeout: 20000 },
  );
  await page.evaluate(() => {
    (window as any).toggleSettings();
    (window as any)._settingsTab('device');
  });
  await page.waitForSelector('#stab-device', { state: 'visible', timeout: 10000 });
}

// Geometry the reported bug is actually about: on screen, sized, not clipped.
async function assertMenuVisibleOnScreen(page, menuSel: string, label: string) {
  const menu = page.locator(menuSel);
  await expect(menu, `${label} menu did not become visible`).toBeVisible();
  const vp = page.viewportSize()!;
  const box = await menu.boundingBox();
  expect(box, `${label} menu has no layout box`).not.toBeNull();
  expect(box!.height, `${label} menu rendered with zero height`).toBeGreaterThan(20);
  expect(box!.width, `${label} menu rendered with zero width`).toBeGreaterThan(40);
  expect(box!.y + box!.height, `${label} menu is above the fold (y=${box!.y})`).toBeGreaterThan(0);
  expect(box!.y, `${label} menu starts below the fold (y=${box!.y}, vh=${vp.height})`).toBeLessThan(vp.height);
  expect(box!.x + box!.width, `${label} menu is off-screen left`).toBeGreaterThan(0);
  expect(box!.x, `${label} menu is off-screen right`).toBeLessThan(vp.width);

  const items = menu.locator('.tab-customizer-item');
  expect(await items.count(), `${label} menu lists no tabs`).toBeGreaterThan(3);
  const row = items.nth(1);
  await expect(row, `a ${label} tab row is not visible`).toBeVisible();
  const ib = await row.boundingBox();
  expect(ib!.height, `a ${label} tab row has zero height`).toBeGreaterThan(10);
  expect(ib!.y + ib!.height, `first ${label} tab row is above the viewport`).toBeGreaterThan(0);
  expect(ib!.y, `first ${label} tab row is below the viewport`).toBeLessThan(vp.height);
}

test('the WORKER tab customizer opens from Settings and its rows are visible on screen', async ({ page }, info) => {
  await openAppearance(page);
  const btn = page.locator('#peek-tab-customize');
  await expect(btn, 'the Worker tabs button is missing from Settings > Appearance').toBeVisible();
  await btn.click();
  await assertMenuVisibleOnScreen(page, '#peek-tab-customizer-menu', 'worker');
  await page.screenshot({ path: `../test-results/tabcust-${info.project.name}.png` });
});

// THE MAIN-NAV customizer, which the test above does NOT cover. Both menus share
// the .tab-customizer-menu class, so verifying one and reporting "the tab
// customizer works" is how the wrong control gets cleared. This one also asserts
// the menu is NOT clipped by the scrolling settings menu it now opens from — the
// reason it was made position:fixed when it moved.
test('the MAIN tab customizer opens from Settings and its rows are visible on screen', async ({ page }, info) => {
  await openAppearance(page);
  const btn = page.locator('#tabs-customize');
  await expect(btn, 'the Main tabs button is missing from Settings > Appearance').toBeVisible();
  await btn.click();
  await assertMenuVisibleOnScreen(page, '#tab-customizer-menu', 'main');

  // Not merely on-screen: NOT CLIPPED by an ancestor's overflow (the settings
  // menu is overflow-y:auto). A fixed menu escapes non-transformed ancestors, so
  // this should pass by construction; it fails loudly if the menu ever regresses
  // to position:absolute inside the settings panel.
  const menu = page.locator('#tab-customizer-menu');
  const clipped = await menu.evaluate((el: Element) => {
    if (getComputedStyle(el).position === 'fixed') return null;
    const r = el.getBoundingClientRect();
    for (let p = el.parentElement; p; p = p.parentElement) {
      const s = getComputedStyle(p);
      if (s.overflow === 'visible' && s.overflowX === 'visible' && s.overflowY === 'visible') continue;
      const pr = p.getBoundingClientRect();
      if (r.bottom > pr.bottom + 1 || r.top < pr.top - 1)
        return `${p.tagName}.${p.className} clips it (menu ${r.top}-${r.bottom} vs parent ${pr.top}-${pr.bottom})`;
    }
    return null;
  });
  expect(clipped, `main menu is clipped by an overflow ancestor: ${clipped}`).toBeNull();

  await page.screenshot({ path: `../test-results/tabcust-global-${info.project.name}.png` });
});
