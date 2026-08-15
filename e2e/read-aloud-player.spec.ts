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

  // Register a one-shot click listener that calls _ttsSpeak from inside the
  // gesture, then click to fire it. WebKit (iOS) rejects audio.play() that is
  // not called inside a user-activation stack; calling _ttsSpeak via a bare
  // page.evaluate has no activation, so _ttsClaimGesture() fails silently and
  // the audio element is left in a state where _abPlay cannot make the bar
  // visible. (Same class as e955a72 — the production fix pre-claims the element;
  // the test fix is to supply the activation the test was missing.)
  //
  // Pattern: start the evaluate WITHOUT awaiting it (so the listener is
  // registered), then click to provide activation and trigger the handler,
  // then await the evaluate's resolution.
  const speakDone = page.evaluate(() =>
    new Promise<void>((resolve, reject) => {
      document.addEventListener('click', async () => {
        try { await (window as any)._ttsSpeak('hello from the sweep', null); resolve(); }
        catch (e) { reject(e); }
      }, { once: true });
    })
  );
  await page.click('body');
  await speakDone;

  // Poll instead of sleeping — a fixed 600ms can expire before the async chain
  // resolves on slow CI machines, and on fast machines it wastes wall-clock time.
  // `(0,eval)`, NOT `window._abEls`. _abEls is a top-level `const` in a classic
  // script (app.js:10696), so it is a global LEXICAL binding and is NOT a
  // property of window — `window._abEls` is undefined, the predicate can never
  // become true, and this poll times out at 30s regardless of whether the bar
  // appeared. The two assertions 6 lines below already use the eval form; the
  // poll was the odd one out.
  //
  // Third instance of this trap in two days (boardItems, _bwWantFrame, now
  // _abEls) and it fails the same way every time: silently, as a timeout that
  // reads like the feature is broken rather than like the probe cannot see it.
  await page.waitForFunction(
    () => {
      try { return !!(0, eval)('_abEls')?.bar?.classList.contains('visible'); }
      catch { return false; }
    },
    { timeout: 5000 }
  );

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
