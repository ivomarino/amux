import { expect, Page, TestInfo, APIRequestContext } from '@playwright/test';
import { captureState } from '../ux-discovery/crawler';

export async function boot(page: Page) {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as any).apiCall === 'function');
  await page.addLocatorHandler(page.locator('#sw-fail-bar'), async bar => {
    await bar.locator('button').last().click();
  });
  const onboarding = page.locator('#wt-overlay.open');
  await onboarding.waitFor({ state: 'visible', timeout: 4000 }).catch(() => {});
  if (await onboarding.isVisible()) await page.locator('#wt-tooltip .wt-skip').click();
}

// Discovery is evidence of visibility, never a claim that a control's effect works.
export async function checkpoint(page: Page, info: TestInfo, name: string) {
  const state = await captureState(page, 0);
  await info.attach(`${name}-controls`, {
    body: JSON.stringify({ ...state, coverage: 'discovered; effects require scenario assertions' }, null, 2),
    contentType: 'application/json',
  });
  await info.attach(name, { body: await page.screenshot({ fullPage: true }), contentType: 'image/png' });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1),
    `${name}: page must fit the viewport`).toBe(true);
}

export async function auth(page: Page) {
  const token = await page.evaluate(() => (window as any)._AMUX_AUTH_TOKEN as string);
  expect(token).toBeTruthy();
  return { Authorization: `Bearer ${token}` };
}

export async function deleteOwnedWorkers(page: Page, request: APIRequestContext,
  headers: Record<string, string>, names: string[]) {
  for (const name of names) {
    expect(name.startsWith('lc-'), 'cleanup must target a run-owned fixture').toBe(true);
    const response = await request.get('/api/sessions', { headers });
    expect(response.ok()).toBeTruthy();
    if (!(await response.json()).some((row: any) => row.name === name)) continue;
    await page.goto('/');
    await page.locator('#tab-sessions').click();
    const card = page.locator(`.card[data-session="${name}"]`).locator('visible=true').first();
    await expect(card).toBeVisible();
    await card.locator('.card-menu-btn').click();
    await page.locator('.card-menu.open [data-worker-action="delete"]').click();
    await expect(page.locator('#modal-msg')).toHaveText(`Delete worker "${name}"?`);
    await page.locator('#modal-btns').getByRole('button', { name: 'Delete', exact: true }).click();
    await expect.poll(async () => {
      const rows = await request.get('/api/sessions', { headers });
      expect(rows.ok()).toBeTruthy();
      return (await rows.json()).some((row: any) => row.name === name);
    }, { message: `UI deletion must actually unregister ${name}` }).toBe(false);
  }
}
