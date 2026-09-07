// Local/Tailscale multiplayer: an owner creates an email-bound invite through
// the real Team UI, a second isolated browser accepts it, then both browsers
// converge on one board and the member's mutation is visible in Amux logs.
import { test, expect } from './fixtures';
import type { Page } from '@playwright/test';

async function settle(page: Page): Promise<void> {
  await expect(page.locator('#conn-status').first()).toBeAttached();
  await page.waitForFunction(() => typeof (window as any).apiCall === 'function');
  await page.addLocatorHandler(page.locator('#sw-fail-bar'), async (bar) => {
    await bar.locator('button').last().click();
  });
  const walkthrough = page.locator('#wt-overlay.open');
  await walkthrough.waitFor({ state: 'visible', timeout: 8_000 }).catch(() => {});
  if (await walkthrough.isVisible()) {
    await page.locator('#wt-tooltip .wt-skip').click();
    await expect(walkthrough).toBeHidden();
  }
}

async function openTeam(page: Page): Promise<void> {
  const menu = page.locator('#settings-menu');
  if (!(await menu.evaluate((el) => el.classList.contains('open')))) {
    await page.click('#settings-btn');
  }
  await expect(menu).toHaveClass(/open/);
  await page.addStyleTag({ content: '#settings-menu .settings-tab-panel{display:block !important}' });
  await expect(page.locator('#settings-team-section')).toBeVisible();
}

test('local invitee joins, shares work, uses worker APIs, appears in logs, and can be revoked', async ({
  page: owner,
  browser,
  request,
}) => {
  await owner.goto('/');
  await settle(owner);
  const ownerToken = await owner.evaluate(() => (window as any)._AMUX_AUTH_TOKEN as string);
  const ownerUiToken = await owner.evaluate(() => (window as any)._AMUX_UI_TOKEN as string);
  expect(ownerToken).toBeTruthy();
  expect(ownerUiToken).toBeTruthy();
  const ownerHeaders = {
    Authorization: `Bearer ${ownerToken}`,
    'Content-Type': 'application/json',
  };

  await openTeam(owner);
  await owner.locator('#settings-team-section button', { hasText: '+ Invite' }).click();
  await expect(owner.locator('#modal-prompt-input')).toBeVisible();
  await owner.locator('#modal-prompt-input').fill('guest@example.com');
  const [inviteResponse] = await Promise.all([
    owner.waitForResponse(
      (response) =>
        response.url().endsWith('/api/org/invites') &&
        response.request().method() === 'POST',
    ),
    owner.locator('#modal-btns button', { hasText: 'OK' }).click(),
  ]);
  expect(inviteResponse.status()).toBe(201);
  const inviteUrl = await owner.locator('#invite-link-input').inputValue();
  expect(inviteUrl).toContain('/invite/');

  // A separate BrowserContext is a separate person: no localStorage, cookies,
  // bearer bootstrap, or service worker state is shared with the owner.
  const guestContext = await browser.newContext({
    ignoreHTTPSErrors: true,
    serviceWorkers: 'block',
  });
  const guest = await guestContext.newPage();
  let memberWorker: string | undefined;
  try {
    await guest.goto(inviteUrl);
    await expect(guest.getByRole('heading', { name: /^Join / })).toBeVisible();
    await expect(guest.locator('#email')).toHaveValue('guest@example.com');
    await guest.locator('#name').fill('Guest User');
    await Promise.all([
      guest.waitForURL((url) => url.pathname === '/'),
      guest.getByRole('button', { name: 'Join workspace' }).click(),
    ]);
    await settle(guest);

    const guestIdentity = await guest.evaluate(async () => {
      const response = await fetch('/api/identity');
      return { status: response.status, body: await response.json() };
    });
    expect(guestIdentity.status).toBe(200);
    expect(guestIdentity.body).toMatchObject({
      email: 'guest@example.com',
      is_local_member: true,
      is_cloud: false,
    });
    expect(
      await guest.evaluate(() => (window as any)._AMUX_AUTH_TOKEN),
      'member shell must use its cookie, never the owner bearer',
    ).toBe('');

    // Both users see the same membership state through their own sessions.
    await openTeam(guest);
    await expect(guest.locator('#settings-members-list')).toContainText('Guest User');
    await owner.locator('#invite-link-input').locator('xpath=ancestor::div[contains(@style,"fixed")]//button[normalize-space()="Done"]').click();
    await openTeam(owner);
    await owner.evaluate(() => (window as any).loadTeamSection());
    await expect(owner.locator('#settings-members-list')).toContainText('Guest User');

    // Multiplayer includes the fleet, not just the board. Exercise the real
    // worker registry and per-worker read surface with only the invitee's
    // HttpOnly cookie. The live Tailnet acceptance run starts and sends to this
    // same shape against a disposable Codex worker; this hermetic case pins the
    // member-auth contract without auto-waking a model in CI.
    memberWorker = `e2e-member-worker-${Date.now()}`;
    const workerAccess = await guest.evaluate(async (workerName) => {
      const create = await fetch('/api/sessions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: workerName,
          dir: '/tmp',
          provider: 'codex',
          creator: 'guest@example.com',
          tags: ['e2e-multiplayer'],
        }),
      });
      const fleet = await fetch('/api/sessions');
      const rows = await fleet.json();
      const info = await fetch(`/api/sessions/${encodeURIComponent(workerName)}/info`);
      return {
        createStatus: create.status,
        fleetStatus: fleet.status,
        listed: rows.some((row: any) => row.name === workerName),
        infoStatus: info.status,
        infoBody: await info.json(),
      };
    }, memberWorker);
    expect(workerAccess).toMatchObject({
      createStatus: 201,
      fleetStatus: 200,
      listed: true,
      infoStatus: 200,
      infoBody: { name: memberWorker },
    });

    // The invitee performs real work with cookie auth. The owner's browser
    // observes the same card after the normal board refresh.
    const title = `local multiplayer ${Date.now()}`;
    const created = await guest.evaluate(async (cardTitle) => {
      const response = await fetch('/api/board', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          title: cardTitle,
          type: 'chore',
          status: 'todo',
          creator: 'guest@example.com',
        }),
      });
      return { status: response.status, body: await response.json() };
    }, title);
    expect(created.status).toBe(201);
    const cardId = created.body.id as string;
    await owner.evaluate(() => {
      document.getElementById('settings-menu')?.classList.remove('open');
      (window as any).switchView('board');
    });
    await owner.evaluate(() => (window as any).fetchBoard());
    await expect
      .poll(
        () =>
          owner.evaluate(
            `(typeof boardItems === 'undefined' ? [] : boardItems).map(i => String(i.title)).includes(${JSON.stringify(title)})`,
          ),
        { timeout: 5_000 },
      )
      .toBe(true);
    // The board defaults to "Working now"; a human-owned unclaimed card is
    // intentionally in Unowned, so select that real view before asserting the
    // owner can see the invitee's card in the UI.
    await owner.locator('.board-view-chip', { hasText: 'Unowned' }).click();
    await expect(owner.locator('#board-view')).toContainText(title);

    // Observability is shared too: the owner can see which member performed
    // the mutation, not merely that some request came from the Tailscale IP.
    await expect
      .poll(
        async () => {
          const response = await request.get(
            '/api/logs?session=' + encodeURIComponent('member:guest@example.com') + '&limit=100',
            { headers: ownerHeaders },
          );
          const payload = await response.json();
          return (payload.events || []).some(
            (event: any) => event.method === 'POST' && event.target === '/api/board',
          );
        },
        { timeout: 5_000 },
      )
      .toBe(true);
    const memberLogView = await guest.evaluate(async () => {
      const response = await fetch(
        '/api/logs?session=' + encodeURIComponent('member:guest@example.com') + '&limit=100',
      );
      const payload = await response.json();
      return {
        status: response.status,
        sawOwnMutation: (payload.events || []).some(
          (event: any) => event.method === 'POST' && event.target === '/api/board',
        ),
      };
    });
    expect(memberLogView).toEqual({ status: 200, sawOwnMutation: true });

    await request.delete(`/api/board/${encodeURIComponent(cardId)}`, { headers: ownerHeaders });
    const members = await (
      await request.get('/api/org/members', { headers: ownerHeaders })
    ).json();
    const member = members.find((entry: any) => entry.email === 'guest@example.com');
    expect(member).toBeTruthy();
    const revoked = await request.delete(`/api/org/members/${encodeURIComponent(member.id)}`, {
      headers: ownerHeaders,
    });
    expect(revoked.status()).toBe(200);
    const afterRevoke = await guest.evaluate(async () => (await fetch('/api/org/members')).status);
    expect(afterRevoke).toBe(401);
    expect(
      await guest.evaluate(
        async (workerName) =>
          (
            await fetch(`/api/sessions/${encodeURIComponent(workerName)}/send`, {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ text: 'must remain revoked' }),
            })
          ).status,
        memberWorker,
      ),
    ).toBe(401);

    // The cookie is HttpOnly and survives member deletion. A full reload must
    // remain revoked; it must never fall through to the public owner shell and
    // receive the owner's bearer just because the member lookup now fails.
    await guest.reload();
    await guest.waitForFunction(() => '_AMUX_AUTH_TOKEN' in window);
    expect(await guest.evaluate(() => (window as any)._AMUX_AUTH_TOKEN)).toBe('');
    const afterReload = await guest.evaluate(async () => (await fetch('/api/org/members')).status);
    expect(afterReload).toBe(401);
  } finally {
    if (memberWorker) {
      await request.delete(`/api/sessions/${encodeURIComponent(memberWorker)}`, {
        headers: { ...ownerHeaders, 'X-Amux-UI-Token': ownerUiToken },
      });
    }
    await guestContext.close();
  }
});
