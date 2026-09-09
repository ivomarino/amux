import { test, expect } from '@playwright/test';
test.beforeEach(async ({ request }) => {
  await request.post('http://127.0.0.1:18824/test/reset');
});
async function home(page: any) {
  await page.goto('/');
  await expect(
    page.getByText('Live connection', { exact: true }),
  ).toBeAttached();
}
async function nav(page: any, name: string) {
  await page
    .locator('[data-slot=sidebar]')
    .getByRole('button', { name, exact: true })
    .click();
}
test('degraded server stays connected with an explicit warning; unknown failures stay unavailable', async ({page, request}) => {
  await request.post('http://127.0.0.1:18824/test/health', {data:{status:'degraded'}});
  await home(page);
  await expect(page.getByText('Amux needs attention.', {exact:false})).toBeVisible();
  await expect(page.getByRole('button', {name:'Create work', exact:true}).first()).toBeEnabled();
  await page.reload();
  await expect(page.getByText('Live connection', {exact:true})).toBeVisible();
  await request.post('http://127.0.0.1:18824/test/health', {data:{status:'unavailable'}});
  await page.getByRole('button', {name:'Refresh workspace'}).click();
  await expect(page.getByText('Connection unavailable', {exact:true})).toBeVisible();
  await expect(page.getByRole('button', {name:'Create work', exact:true}).first()).toBeDisabled();
});
test('home shows source-derived business state and restrained design', async ({
  page,
}) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await home(page);
  await expect(
    page.getByRole('heading', { name: 'Today', exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText('Northwind Supply — payment needs matching').first(),
  ).toBeVisible();
  await expect(page.getByText('Not measured in this workspace')).toBeVisible();
  await expect(
    page.getByRole('heading', { name: 'Receivables review', exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: 'test-results/business-desktop.png',
    fullPage: true,
  });
  expect(errors).toEqual([]);
});
test('work filters, search and evidence drawer', async ({ page }) => {
  await home(page);
  await nav(page, 'Work');
  await page.getByRole('tab', { name: 'All work', exact: true }).click();
  await page.getByRole('textbox', { name: 'Search work' }).fill('Harbor');
  await expect(page.locator('.queue-row')).toHaveCount(1);
  await page.locator('.queue-row').click();
  await expect(
    page.getByRole('heading', {
      name: 'Harbor Services — invoices reconciled',
    }),
  ).toBeVisible();
  await expect(
    page.getByText('Verification passed', { exact: true }),
  ).toBeVisible();
  await expect(page.locator('[data-slot=artifact-card]')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(
    page.getByRole('heading', { name: 'Work', exact: true }),
  ).toBeVisible();
});
test('new work survives reload and exists in the same Amux board', async ({
  page,
  request,
}) => {
  await home(page);
  await page
    .getByRole('button', { name: 'Create work', exact: true })
    .first()
    .click();
  await page
    .getByLabel('What needs to be done?')
    .fill('Review August receipts');
  await page
    .getByLabel('Instructions and context')
    .fill('Find missing references and return an evidence-linked report.');
  await page
    .getByRole('dialog')
    .getByRole('button', { name: 'Create work', exact: true })
    .click();
  await expect(
    page.getByRole('heading', { name: 'Review August receipts' }),
  ).toBeVisible();
  await page.keyboard.press('Escape');
  await page.reload();
  await nav(page, 'Work');
  await page.getByRole('tab', { name: 'To plan' }).click();
  await expect(
    page.getByText('Review August receipts', { exact: true }),
  ).toBeVisible();
  const rows = await (
    await request.get('http://127.0.0.1:18824/api/board')
  ).json();
  expect(
    rows.find((t: any) => t.title === 'Review August receipts').status,
  ).toBe('backlog');
});
test('assistant-ui request is a durable work item rather than chat-only state', async ({
  page,
  request,
}) => {
  await home(page);
  await page.getByRole('button', { name: 'Ask Amux', exact: true }).click();
  await page
    .getByRole('dialog')
    .getByRole('textbox', { name: 'What would you like Amux to do?' })
    .fill('Find missing supplier documents');
  await page
    .getByRole('dialog')
    .getByRole('button', { name: 'Add request to work' })
    .click();
  await expect(
    page
      .locator('[data-slot=artifact-card]')
      .filter({ hasText: 'Find missing supplier documents' }),
  ).toBeVisible();
  await expect(
    page.getByText('I added this to Work, ready to plan.', { exact: false }),
  ).toBeVisible();
  const rows = await (
    await request.get('http://127.0.0.1:18824/api/board')
  ).json();
  expect(
    rows.some((t: any) => t.title === 'Find missing supplier documents'),
  ).toBe(true);
});
test('approval requires complete preview and explicit confirmation; a decision sends once', async ({
  page,
  request,
}) => {
  await home(page);
  await nav(page, 'Approvals');
  await page
    .locator('main')
    .getByRole('button', { name: /Payment reference/ })
    .click();
  await expect(
    page.getByText(
      'Hello Alex, could you confirm which invoice your recent payment covers? Thank you.',
      { exact: true },
    ),
  ).toBeVisible();
  const approve = page.getByRole('button', { name: 'Approve and send' });
  await expect(approve).toBeDisabled();
  await page.getByRole('checkbox').check();
  await approve.click();
  await expect(
    page.getByText('Amux accepted the send.', { exact: false }),
  ).toBeVisible();
  const requests = await (
    await request.get('http://127.0.0.1:18824/test/requests')
  ).json();
  expect(
    requests.filter((r: any) => r.path.includes('/email/approve/')),
  ).toHaveLength(1);
  expect(
    requests.find((r: any) => r.path.includes('/email/approve/')).approver,
  ).toBe('business-ui');
});
test('legacy incomplete previews cannot be approved', async ({
  page,
  request,
}) => {
  await request.post('http://127.0.0.1:18824/test/incomplete');
  await home(page);
  await nav(page, 'Approvals');
  await page
    .locator('main')
    .getByRole('button', { name: /Payment reference/ })
    .click();
  await expect(
    page.getByRole('button', { name: 'Approve and send' }),
  ).toBeDisabled();
  await expect(
    page.getByText('The server has not supplied a complete', { exact: false }),
  ).toBeVisible();
});
test('expired approval cannot be released; rejection is recorded', async ({
  page,
  request,
}) => {
  await request.post('http://127.0.0.1:18824/test/expire');
  await home(page);
  await nav(page, 'Approvals');
  await page
    .locator('main')
    .getByRole('button', { name: /Payment reference/ })
    .click();
  await expect(
    page.getByRole('button', { name: 'Approve and send' }),
  ).toBeDisabled();
  await page.getByRole('button', { name: 'Reject', exact: true }).click();
  await expect(
    page.getByText('Rejected. This action was not released.'),
  ).toBeVisible();
});
test('operator review note persists to Amux with revision check', async ({
  page,
  request,
}) => {
  await page.route(/\/(tasks|board)\/TEST-1$/, async route => {
    if (route.request().method() === 'GET') await new Promise(resolve => setTimeout(resolve, 800));
    await route.continue();
  });
  await home(page);
  await page
    .getByRole('button', { name: /Northwind Supply — payment needs matching/ })
    .first()
    .click();
  await expect(page.getByLabel('Reviewer name')).toBeDisabled();
  await page
    .getByLabel('Add a note')
    .fill('Confirmed invoice INV-1004 with the remittance advice.');
  await page.getByLabel('Reviewer name').fill('Jamie');
  await page.getByRole('button', { name: 'Save review note' }).click();
  await expect(page.getByLabel('Add a note')).toHaveValue('');
  const t = await (
    await request.get('http://127.0.0.1:18824/api/board/TEST-1')
  ).json();
  expect(t.desc).toContain('Confirmed invoice INV-1004');
  expect(t.reviewer).toBe('Jamie');
});
test('workflow is configuration of existing operations and schedule pause uses Amux', async ({
  page,
  request,
}) => {
  await home(page);
  await nav(page, 'Automations');
  await page
    .locator('article')
    .filter({
      has: page.getByRole('heading', {
        name: 'Receivables review',
        exact: true,
      }),
    })
    .getByRole('button', { name: 'Details' })
    .click();
  await page.getByRole('button', { name: 'Pause schedule' }).click();
  await expect(
    page.getByRole('button', { name: 'Resume schedule' }),
  ).toBeVisible();
  const sched = await (
    await request.get('http://127.0.0.1:18824/api/schedules')
  ).json();
  expect(sched[0].enabled).toBe(false);
  await page.keyboard.press('Escape');
  await page
    .getByRole('button', { name: 'Create automation', exact: true })
    .click();
  await page.getByLabel('Automation name').fill('Vendor onboarding');
  await page
    .getByLabel('What does it take care of?')
    .fill('Collects vendor documentation.');
  await page.getByRole('button', { name: 'Continue', exact: true }).click();
  await page
    .getByRole('checkbox', { name: 'Document collection operation documents', exact: true })
    .check();
  await page.getByRole('button', { name: 'Save automation' }).click();
  await expect(
    page.getByRole('heading', { name: 'Vendor onboarding', exact: true }),
  ).toBeVisible();
});
test('app connection check uses existing connector API', async ({
  page,
  request,
}) => {
  await home(page);
  await nav(page, 'Apps');
  await page
    .locator('article')
    .filter({ has: page.getByRole('heading', { name: 'Gmail', exact: true }) })
    .getByRole('button', { name: 'Manage connection' })
    .click();
  await page.getByRole('button', { name: 'Check connection' }).click();
  await expect(
    page.getByText('Connection verified by the provider.'),
  ).toBeVisible();
});
test('mobile navigation, readable controls, no horizontal overflow', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await home(page);
  await expect(
    page.getByRole('navigation', { name: 'Mobile navigation' }),
  ).toBeVisible();
  await page
    .getByRole('navigation', { name: 'Mobile navigation' })
    .getByRole('button', { name: 'Work', exact: true })
    .click();
  await expect(
    page.getByRole('heading', { name: 'Work', exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page
    .getByRole('navigation', { name: 'Mobile navigation' })
    .getByRole('button', { name: 'Home' })
    .click();
  await page.screenshot({
    path: 'test-results/business-mobile.png',
    fullPage: true,
  });
});
test('200% text resize remains usable without clipping page', async ({
  page,
}) => {
  await home(page);
  await page.addStyleTag({ content: 'html{font-size:32px}' });
  await expect(
    page.getByRole('heading', { name: 'Today', exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});
test('transport blocks cross-origin writes, arbitrary API paths and unconfirmed approval', async ({
  request,
}, testInfo) => {
  test.skip(testInfo.project.name==='embedded', 'This test exercises the development-only gateway; native API authority is covered by Rust tests.');
  expect(
    (
      await request.post('/api/business/tasks', {
        headers: { Origin: 'https://evil.example' },
        data: { title: 'bad', description: '' },
      })
    ).status(),
  ).toBe(403);
  expect(
    (await request.get('/api/business/../business/secrets')).status(),
  ).toBe(404);
  expect(
    (
      await request.post(
        '/api/business/approvals/email/apr_0123456789abcdef/approve',
        { data: {} },
      )
    ).status(),
  ).toBe(400);
});
