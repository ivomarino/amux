import {readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
import vm from 'node:vm';
import test from 'node:test';
import assert from 'node:assert/strict';
const require=createRequire(import.meta.url);
const {parse}=require('espree');
// The override runs this same contract against a committed pre-fix specimen.
const source=readFileSync(process.env.AMUX_TERMINAL_RENDER_SOURCE || new URL('../crates/amux-dashboard/static/app.js',import.meta.url),'utf8');
const ast=parse(source,{ecmaVersion:'latest',range:true});
function fixture() {
  const context=vm.createContext({console, performance, Map, Promise, JSON, _peekRenderCache:new Map(),
    _peekRenderWork:{parsed_chars:0,parsed_chunks:0,dom_chunks:0,paints:0,coalesced:0},
    _MSG_KIND:{unknown:{label:'Unknown'}}, _classifyPromptKind:()=> 'unknown',
    _linkifyPaths:s=>s, rewriteLocalhostUrls:s=>s,
    esc:s=>s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;'),
    document:{createElement:()=>({innerHTML:'',get value(){return this.innerHTML.replace(/&lt;/g,'<').replace(/&gt;/g,'>').replace(/&amp;/g,'&');}})},
  });
  for (const name of ['ansiToHtml','_osc8Resolve','_peekHtml','_peekPromptNormalized','highlightPrompts','wrapBoxBlocks','_fitRules','_peekRenderChunks','_peekChunkHTML','_queuePeekFrame']) {
    const node=ast.body.find(n=>n.type==='FunctionDeclaration' && n.id.name===name);
    assert.ok(node,name+' must be shipped'); vm.runInContext(source.slice(...node.range),context);
  }
  // Only historical specimens contain these dependencies. Load them when
  // present so a red counterexample tests their behavior, not a missing name.
  for (const name of ['_peekHtmlSlice','_peekCodeRows']) {
    const node=ast.body.find(n=>n.type==='FunctionDeclaration' && n.id.name===name);
    if (node) vm.runInContext(source.slice(...node.range),context);
  }
  return context;
}
const rows=n=>Array.from({length:n},(_,i)=>` ${String(i+1).padStart(5)}- old source   │ ${String(i+1).padStart(5)}+ new source\n`).join('');
test('grep context prefixes remain literal text without inferred deleted rows',()=>{
  const c=fixture();
  const raw='38-    background: #1c2128;\n39-    color: #fff;\n40:    margin: 0;\n';
  assert.equal(c._peekHtml(raw),raw);
  assert.equal(c._peekChunkHTML(c._peekRenderChunks('history',raw)).replace(/<[^>]*>/g,''),raw);
});
test('source escaping preserves its actual ANSI color without inferring change styling',()=>{
  const c=fixture(), html=c._peekHtml('\x1b[32m  12+ <tag> & contents\n  13+ const value = 1;\x1b[0m');
  assert.match(html,/&lt;tag&gt; &amp; contents/);
  assert.equal((html.match(/<span[ >]/g)||[]).length,(html.match(/<\/span>/g)||[]).length);
  assert.match(html,/<span style="color:#4e9a06">  12\+ /);
  assert.doesNotMatch(html,/peek-code-|background:/);
});
test('appending to a megabyte transcript parses only the final block and keeps prior identities',()=>{
  const c=fixture(), raw=rows(16000), first=c._peekRenderChunks('history',raw);
  const before=c._peekRenderWork.parsed_chars;
  const next=c._peekRenderChunks('history',raw+' 16001+ last source\n 16002+ new source\n');
  assert.equal(next[0],first[0]); assert.equal(next[100],first[100]);
  assert.ok(c._peekRenderWork.parsed_chars-before<6000);
  const unchanged=c._peekRenderWork.parsed_chars;
  c._peekRenderChunks('history',raw+' 16001+ last source\n 16002+ new source\n');
  assert.equal(c._peekRenderWork.parsed_chars,unchanged);
});
test('ANSI color and OSC hyperlinks carry through stable chunk boundaries and resets',()=>{
  const c=fixture(), raw='\x1b[31m\x1b]8;;https://example.test/source\x07'+('colored linked row\n'.repeat(140))+'\x1b]8;;\x07\x1b[0mplain\n';
  const chunks=c._peekRenderChunks('live',raw);
  assert.ok(chunks.length>=3);
  assert.match(chunks[1].html,/color:#cc0000/);
  assert.match(chunks[1].html,/href="https:\/\/example.test\/source"/);
  assert.match(chunks.at(-1).html,/<\/span>plain/);
});
test('a rewritten capture invalidates only affected chunks, including carried style',()=>{
  const c=fixture(), raw=rows(200), first=c._peekRenderChunks('live',raw);
  const next=c._peekRenderChunks('live',raw.replace('old source','OLDER data'));
  assert.notEqual(next[0],first[0]); assert.equal(next[1],first[1]);
  const colored=c._peekRenderChunks('live','\x1b[31m'+raw);
  assert.notEqual(colored[1],next[1]); assert.match(colored[1].html,/color:#cc0000/);
});
test('chunk boundaries never turn a continued human prompt into separate messages',()=>{
  const c=fixture(), raw='preamble\n'.repeat(63)+'› long request\n'+('  continued paragraph\n'.repeat(140))+'Assistant reply\n';
  const html=c._peekChunkHTML(c._peekRenderChunks('history',raw));
  assert.equal((html.match(/data-msg-kind=/g)||[]).length,1);
  assert.ok(html.indexOf('continued paragraph')>html.indexOf('data-msg-kind='));
});

test('numbers, aligned gaps, and a column divider are not sufficient diff evidence',()=>{
  const c=fixture();
  for (const raw of ['  9+ ordinary command output',
    '  1- old        1+ new\n  2- old        2+ new\n',
    '  1+ "literal    7+ inside"\n  2+ other output\n', rows(2)]) {
    assert.equal(c._peekHtml(raw),raw);
  }
});

test('coalesced frame failures reject callers, announce themselves, and allow retry',async()=>{
  const c=fixture(), scheduled=[], reports=[];
  Object.assign(c,{_peekFramePending:null, _peekIdentityCurrent:()=>true,
    requestAnimationFrame:f=>scheduled.push(f), API:'', APP_VER:'test',
    fetch:(_url,options)=>{reports.push(JSON.parse(options.body));return Promise.resolve();},
    _peekAcceptFrame:()=>{throw new Error('malformed frame');}});
  const one=c._queuePeekFrame({name:'worker',generation:1},{live:'x'});
  const two=c._queuePeekFrame({name:'worker',generation:1},{live:'y'});
  const rejected=Promise.all([assert.rejects(one,/malformed/),assert.rejects(two,/malformed/)]);
  assert.equal(scheduled.length,1); scheduled.shift()(); await rejected;
  assert.equal(reports[0].verdict,'render-failed'); assert.equal(reports[0].measured,true);
  c._peekAcceptFrame=data=>{c.painted=data.live;};
  const retry=c._queuePeekFrame({name:'worker',generation:1},{live:'recovered'});
  scheduled.shift()(); await retry; assert.equal(c.painted,'recovered');
});
