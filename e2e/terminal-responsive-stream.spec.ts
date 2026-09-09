import { test, expect, type Page } from './fixtures';

async function setup(page: Page) {
  await page.addInitScript(() => localStorage.setItem('amux_walkthrough_done', '1'));
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as any)._peekHtml === 'function');
  await page.evaluate(() => {
    eval("peekSession='stream-fixture'; _peekMsgRowsFor=peekSession; _peekMsgRows=[]; _peekMsgNavKind='all'; _peekMsgIndex=-1; _peekHistoryHTML=''; _peekHistoryRaw=''; _lastPeekRaw=''; lastPeekHTML=''; _peekScrollLocked=false; peekSearchQuery=''; _peekEarlier={chunks:[],loadedKb:0,done:true,hidden:true,loading:false};");
    document.getElementById('peek-overlay')!.classList.add('active');
    (window as any)._stopPeekPoll();
    document.getElementById('peek-title')!.textContent='Streaming renderer fixture';
    // Isolated synthetic stream. Never ask a live worker for its transcript.
  });
  await expect(page.locator('#peek-overlay')).toHaveCSS('opacity','1');
}
function diff(count: number, from=1) {
  return Array.from({length:count}, (_, i) => {
    const n = String(i+from).padStart(5);
    return ` ${n}- const before = "long old source with <tags> & a path /tmp/example.rs";   │ ${n}+ const after = "readable source with a much longer value and attached line number";\n`;
  }).join('');
}
async function frame(page: Page, history: string | null, live = 'Working…\n') {
  await page.evaluate(async ({history, live}) => {
    const w = window as any;
    await w._queuePeekFrame(w._peekIdentity(), {name:'stream-fixture', history, live});
  }, {history, live});
}

for (const width of [390, 1280]) {
  test(`plain grep output keeps its text and columns at ${width}px`, async ({page}) => {
    await page.setViewportSize({width,height:844});
    await setup(page);
    const raw='38-    background: #1c2128;\n39-    color: #fff;\n40:    margin: 0;\n'
      +'  1- old        1+ new\n  2- old        2+ new\n'
      +'  3- literal <tag> & text   │  3+ another command column\n';
    await frame(page,raw);
    expect(await page.locator('#pk-hist').textContent()).toBe(raw);
    const rendered=await page.locator('#pk-hist').evaluate(el => ({
      splits:[...el.querySelectorAll('*')].filter(node => getComputedStyle(node).display==='grid').length,
      tints:[...el.querySelectorAll('*')].filter(node => !['transparent','rgba(0, 0, 0, 0)'].includes(getComputedStyle(node).backgroundColor)).length,
    }));
    expect(rendered).toEqual({splits:0,tints:0});
    await page.screenshot({path:test.info().outputPath(`plain-grep-${width}.png`)});
  });

  test(`scroll-lock transitions do not move terminal controls or content at ${width}px`, async ({page}) => {
    await page.setViewportSize({width,height:844});
    await setup(page);
    await frame(page,'ordinary terminal row\n'.repeat(300));
    const samples=await page.evaluate(async () => {
      const w=window as any, body=document.getElementById('peek-body')!;
      const controls=document.querySelector('.peek-output-controls')!;
      body.scrollTop=500;
      const measure=() => {
        const badge=controls.querySelector('.scroll-lock-badge');
        const style=badge ? getComputedStyle(badge) : null;
        return {top:body.getBoundingClientRect().top,height:body.getBoundingClientRect().height,
          controls:controls.getBoundingClientRect().height,scroll:body.scrollTop,
          // A short English label can fit without changing height today. It
          // must still never participate in toolbar flow: wrapping/zoom turns
          // that placement into a resize on every show/hide transition.
          badgeInToolbarFlow:!!style && style.display!=='none' && !['absolute','fixed'].includes(style.position)};
      };
      const observed=[measure()];
      for (let i=0;i<12;i++) {
        eval('_peekScrollLocked = ' + (i%2===0));
        if (i%2===0) w._showScrollLockBadge(body,()=>{}); else w._hideScrollLockBadge(body);
        await new Promise<void>(resolve => requestAnimationFrame(()=>requestAnimationFrame(()=>resolve())));
        observed.push(measure());
      }
      return observed;
    });
    for (const sample of samples) {
      expect(sample.badgeInToolbarFlow).toBe(false);
      for (const key of ['top','height','controls','scroll'] as const) {
        expect(Math.abs(sample[key]-samples[0][key]),key).toBeLessThan(1);
      }
    }
    console.log('[scroll-lock geometry]',JSON.stringify({width,samples}));
  });
}

test('composer chips retain horizontal-only touch handling', async ({page}) => {
  await page.setViewportSize({width:390,height:844});
  await setup(page);
  await expect(page.locator('#peek-chips')).toHaveCSS('touch-action','pan-x');
});

test('large streaming history reuses DOM, coalesces bursts, and preserves typing and scroll', async ({page}) => {
  await page.setViewportSize({width:390,height:844});
  await setup(page);
  const history = diff(6000);
  await frame(page, history);
  const initial = await page.evaluate(() => {
    const w = window as any, body = document.getElementById('peek-body')!;
    w._firstStreamChunk = document.querySelector('#pk-hist > div');
    body.scrollTop = 1800;
    eval('_peekScrollLocked = true;');
    return {work:eval('({..._peekRenderWork})'), nodes:body.querySelectorAll('*').length, top:body.scrollTop};
  });
  await page.locator('#peek-cmd-input').fill('Draft survives streaming');
  const after = await page.evaluate(async history => {
    const w = window as any;
    const before = eval('({..._peekRenderWork})'), start = performance.now();
    await Promise.all(Array.from({length:30}, (_, i) => w._queuePeekFrame(w._peekIdentity(),
      {name:'stream-fixture', history:null, live:'Streaming update ' + i + '\n'})));
    await w._queuePeekFrame(w._peekIdentity(), {name:'stream-fixture', history:history+' 6001+ new source line\n 6002+ new source line\n',live:'Latest output\n'});
    return {before, work:eval('({..._peekRenderWork})'), duration:performance.now()-start,
      same:document.querySelector('#pk-hist > div') === w._firstStreamChunk,
      nodes:document.getElementById('peek-body')!.querySelectorAll('*').length,
      top:document.getElementById('peek-body')!.scrollTop};
  }, history);
  expect(after.work.paints-after.before.paints).toBe(2);
  expect(after.work.coalesced-after.before.coalesced).toBe(29);
  expect(after.work.parsed_chars-after.before.parsed_chars).toBeLessThan(20000);
  expect(after.work.dom_chunks-after.before.dom_chunks).toBe(0); // scrolled reader buffers output
  expect(after.same).toBe(true);
  expect(Math.abs(after.top-initial.top)).toBeLessThan(2);
  await expect(page.locator('#peek-cmd-input')).toHaveValue('Draft survives streaming');
  const resume = await page.evaluate(() => {
    const w = window as any;
    const before=eval('({..._peekRenderWork})');
    eval('_peekScrollLocked=false;');
    w.applyPeekSearch(false,false);
    return {work:eval('({..._peekRenderWork})'),before,same:document.querySelector('#pk-hist > div')===w._firstStreamChunk,
      nodes:document.getElementById('peek-body')!.querySelectorAll('*').length};
  });
  expect(resume.same).toBe(true);
  expect(resume.work.dom_chunks-resume.before.dom_chunks).toBeLessThanOrEqual(3);
  expect(resume.nodes-initial.nodes).toBeLessThan(40);
  expect(resume.nodes).toBeLessThan(6000*9+300); // retain the original work bound; no duplicate transcript
  console.log('[terminal render measurement]', JSON.stringify({initial,after,resume}));
});

test('streaming at the bottom bounds DOM writes and lets input and frame callbacks run', async ({page}) => {
  await page.setViewportSize({width:390,height:844});
  await setup(page);
  const history=diff(3000);
  await frame(page,history);
  await page.evaluate(() => {
    const w=window as any, body=document.getElementById('peek-body')!;
    body.scrollTop=body.scrollHeight;
    eval('_peekScrollLocked=false;');
    w._streamFirst=document.querySelector('#pk-hist > div');
    w._streamBefore=eval('({..._peekRenderWork})');
    w._streamDurations=[];
  });
  // Keep a real textarea focused while the shipped update path appends history.
  await page.locator('#peek-cmd-input').focus();
  for (let i=0;i<8;i++) {
    await page.keyboard.type(String(i));
    await page.evaluate(async ({history,i}) => {
      const w=window as any, start=performance.now();
      await w._queuePeekFrame(w._peekIdentity(),{name:'stream-fixture',history:history+(' 3001+ appended source\n'.repeat(i+1)),live:'Active tick '+i});
      w._streamDurations.push(performance.now()-start);
    }, {history,i});
  }
  await expect(page.locator('#peek-cmd-input')).toHaveValue('01234567');
  const metrics=await page.evaluate(() => {
    const w=window as any;
    return {before:w._streamBefore,after:eval('({..._peekRenderWork})'),durations:w._streamDurations,
      same:document.querySelector('#pk-hist > div')===w._streamFirst};
  });
  expect(metrics.same).toBe(true);
  expect(metrics.after.dom_chunks-metrics.before.dom_chunks).toBeLessThanOrEqual(24);
  expect(metrics.after.parsed_chars-metrics.before.parsed_chars).toBeLessThan(100000);
  console.log('[active terminal render measurement]',JSON.stringify(metrics));
});

test('a queued frame from an old worker cannot replace the current worker or its history', async ({page}) => {
  await setup(page);
  const result=await page.evaluate(async () => {
    const w=window as any, old=w._peekIdentity();
    const first=w._queuePeekFrame(old,{name:'stream-fixture',history:'OLD SECRET HISTORY',live:'old output'});
    eval("peekSession='new-fixture'; _peekOpenGeneration++; _peekRenderCache.clear();");
    const current=w._queuePeekFrame(w._peekIdentity(),{name:'new-fixture',history:'current history',live:'current output'});
    const late=w._queuePeekFrame(old,{name:'stream-fixture',history:'OLD LATE HISTORY',live:'late output'});
    await Promise.all([first,current,late]);
    return document.getElementById('peek-body')!.textContent;
  });
  expect(result).toContain('current history');
  expect(result).toContain('current output');
  expect(result).not.toContain('OLD');
});
