import { test, expect } from '@playwright/test';
test('read-aloud plays through the shared bottom player', async ({ page }) => {
  await page.route('**/api/tts', r => r.fulfill({ contentType: 'application/json',
    body: JSON.stringify({ engine: 'stub', size: 44,
      url: 'data:audio/wav;base64,UklGRiQAAABXQVZFZm10IBAAAAABAAEARKwAAIhYAQACABAAZGF0YQAAAAA=' }) }));
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as any)._ttsSpeak === 'function', { timeout: 20000 });
  const before = await page.evaluate(() =>
    document.querySelector('#amux-audio-bar,.audio-bar,[id*="audio"][class*="bar"]')?.className || 'NO BAR ELEMENT');
  console.log('[BAR BEFORE] ' + before);
  await page.evaluate(async () => { await (window as any)._ttsSpeak('hello from the sweep', null); });
  await page.waitForTimeout(600);
  const r = await page.evaluate(() => {
    const a = (0, eval)('_abAudio');
    const bar = (0, eval)('_abEls').bar;
    return { barVisible: bar.classList.contains('visible'),
             sharedElementUsed: (0, eval)('_ttsSpeakAudio') === a,
             srcIsClip: (a.src || '').startsWith('data:audio/wav'),
             title: (0, eval)('_abEls').title.textContent };
  });
  console.log('[AFTER] ' + JSON.stringify(r));
  expect(r.barVisible, 'bottom player bar did not become visible').toBe(true);
  expect(r.sharedElementUsed, 'played on a detached element, not the shared player').toBe(true);
  expect(r.title, 'player title should name the feature').toMatch(/Read aloud/);
});
