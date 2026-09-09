import { test, expect } from './fixtures';

// A DEAD TUNNEL HANGS; IT DOES NOT REFUSE.
//
// Reported by Ethan 2026-09-08: a client reaching amux over Tailscale "is
// unable to send or receive anything new. it just stores everything that's
// been cached and presents it as if it's new."
//
// The offline detector latches on `consecutiveFailures >= 2`, and a FAILURE is
// something a fetch has to return. A refused connection returns one. A tunnel
// whose far end has gone away does not: the request is accepted by the local
// stack and simply never answered. `navigator.onLine` also stays true, because
// the DEVICE still has a network. So the two inputs that could say "this is
// stale" both report healthy, and the last successful render stays on screen
// wearing no mark.
//
// This is the AMUX-2585 shape one turn further out. There the outbox's own
// synthetic 202 kept resetting the failure counter; here nothing resets it,
// nothing increments it either, and the counter is simply never consulted.
//
// The test black-holes the API the way the tunnel does — accepted, never
// answered — and asks only that the UI STOP CLAIMING TO BE LIVE. It does not
// prescribe the remedy.
test('a hung API (dead tunnel) must not keep reading as live', async ({ page }) => {
  // The SSE zombie threshold alone is 18s, so this cell needs headroom the
  // default 30s test timeout does not have.
  test.setTimeout(120_000);
  await page.goto('/');
  await page.waitForLoadState('networkidle');

  // Assert on what the USER sees. #conn-status is the indicator the dashboard
  // renders: 'Live', 'Polling', 'Offline', or '<n> pending'.
  const indicator = page.locator('#conn-status').first();
  await expect(indicator, 'precondition: the client starts connected').toHaveText(
    /Live|Polling/, { timeout: 20000 });
  const before = (await indicator.textContent())?.trim();

  // Accepted and never answered. Deliberately NOT route.abort(): an abort is a
  // failure the client can count, which is the case that already works.
  let released = false;
  await page.route('**/api/**', async () => {
    await new Promise<void>(resolve => {
      const tick = setInterval(() => { if (released) { clearInterval(tick); resolve(); } }, 250);
    });
  });

  // AND SEVER THE STREAM THAT IS ALREADY OPEN. `page.route` only intercepts NEW
  // requests, so the EventSource opened during load survives it and keeps
  // delivering the server's 10s pings — the client stays genuinely live and the
  // badge is genuinely right. The first draft of this test missed that and
  // "reproduced" a bug it had not created.
  //
  // A dying tunnel takes the open stream with it, so drop it here. The
  // reconnect then falls into the black hole above, which is the real state:
  // no stream, no answering fetches, and a device that still has a network.
  await page.evaluate(() => (window as any)._forceSseReconnect('e2e tunnel blackhole'));

  // MEASURE FIRST. Sample the indicator across the window and report every
  // distinct value, so the assertion is written against what the client
  // actually does rather than what it was assumed to do.
  const seen: string[] = [];
  for (let i = 0; i < 12; i++) {
    const t = (await indicator.textContent())?.trim() || '';
    if (t !== seen[seen.length - 1]) seen.push(t);
    await page.waitForTimeout(4000);
  }

  released = true;
  await page.unroute('**/api/**');

  const after = seen[seen.length - 1];
  console.log(`[tunnel-blackhole] indicator over 48s of a hung API: ${JSON.stringify(seen)}`);

  // THE CLAIM UNDER TEST. With nothing getting through, the badge must not
  // claim a working connection. `Live` is one such claim and `Polling` is the
  // other: it asserts a fallback that is fetching, and under a hung tunnel the
  // fetches never return, so nothing counts a failure and nothing latches
  // offline. What is on screen is the last thing we heard.
  expect(
    after,
    `after 48s with every /api/ request accepted and never answered, and the ` +
    `open SSE stream severed, the indicator reads ${JSON.stringify(after)} ` +
    `(sequence: ${JSON.stringify(seen)}). A hang returns no failure for ` +
    `consecutiveFailures to count and navigator.onLine stays true, so the ` +
    `client cannot reach the state that would mark the data stale.`
  ).not.toMatch(/^(Live|Polling)$/);
});
