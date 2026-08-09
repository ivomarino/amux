// Phase 0 golden scenarios (RR-0025): server boots, dashboard loads, health
// is truthful, auth rejects bad tokens, mobile renders without overflow.
import { test, expect } from '@playwright/test';

test('health returns 200 with build hash and revision', async ({ request }) => {
  const res = await request.get('/health');
  expect(res.status()).toBe(200);
  const body = await res.json();
  expect(body.status).toBe('ok');
  expect(body.server).toBe('amux-rust');
  expect(typeof body.build).toBe('string');
  expect(body.build.length).toBeGreaterThan(0);
  expect(typeof body.rev).toBe('number');
});

test('dashboard shell loads with no console errors', async ({ page }) => {
  const errors: string[] = [];
  page.on('console', (msg) => {
    if (msg.type() === 'error') errors.push(msg.text());
  });
  await page.goto('/');
  await expect(page.getByTestId('dashboard-shell')).toBeVisible();
  // The shell's own health probe must succeed — proves static serving and
  // API serving coexist on one port.
  await expect(page.getByTestId('health-status')).toContainText('server ok');
  expect(errors).toEqual([]);
});

test('viewport renders without horizontal overflow', async ({ page }) => {
  // Runs in both projects; the mobile project (375px) is the load-bearing
  // case — amux is mobile-first.
  await page.goto('/');
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth > document.documentElement.clientWidth,
  );
  expect(overflow).toBe(false);
});

test('protected API rejects a bad bearer token', async ({ request }) => {
  const bad = await request.get('/api/sync?since_rev=0', {
    headers: { Authorization: 'Bearer wrong-token' },
  });
  expect(bad.status()).toBe(401);
});

test('protected API rejects a missing token', async ({ request }) => {
  const res = await request.get('/api/sync?since_rev=0');
  expect(res.status()).toBe(401);
});
