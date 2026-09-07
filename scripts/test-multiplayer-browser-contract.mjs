#!/usr/bin/env node

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const root = new URL('../', import.meta.url);
const read = path => readFileSync(new URL(path, root), 'utf8');

const app = read('crates/amux-dashboard/static/app.js');
const serviceWorker = read('crates/amux-dashboard/static/sw.js');
assert.ok(serviceWorker.includes("url.searchParams.has('_token')"),
  'the service worker must send remote-owner token exchange navigations to the server');
assert.ok(serviceWorker.includes('e.respondWith(fetch(e.request))'),
  'the remote-owner token exchange must preserve the original request and redirect cookies');
const appVersion = app.match(/const APP_VER = '([^']+)'/)?.[1];
const cacheVersion = serviceWorker.match(/const CACHE = 'amux-v([^']+)'/)?.[1];
assert.equal(appVersion, cacheVersion, 'app and service-worker cache versions must move together');
const skipLine = app.split('\n').find(line => line.startsWith('const _OUTBOX_SKIP = '));
assert.ok(skipLine, 'dashboard must define the offline-outbox exclusion list');
const skip = Function(`return ${skipLine.slice(skipLine.indexOf('=') + 1, skipLine.lastIndexOf(';'))}`)();
assert.ok(skip.test('/api/gateway/switch-org'),
  'workspace switches must never be replayed later from the offline outbox');

const switchStart = app.indexOf('async function _switchOrg');
const switchEnd = app.indexOf('\n}\n', switchStart) + 2;
const switchSource = app.slice(switchStart, switchEnd);
for (const required of [
  "'Accept':'application/json'",
  '_skipOutbox: true',
  '_isLocallyQueued(r)',
  "if (ack?.ok !== true)",
  'setTimeout(() => ctl.abort(), 20000)',
  'workspace_switch_failed',
  'still viewing the current workspace',
]) {
  assert.ok(switchSource.includes(required), `workspace switch contract missing: ${required}`);
}

const gateway = read('cloud/gateway/gateway.py');
for (const required of [
  'wants_json = "application/json" in self.headers.get("Accept", "")',
  'extra_cookies=[cookie]',
  'verdict=switched',
  '"via_god_mode": bool(is_admin and r["membership_user_id"] is None)',
]) {
  assert.ok(gateway.includes(required), `gateway workspace contract missing: ${required}`);
}

const cdp = read('skills/chrome-cdp/scripts/cdp.mjs');
for (const required of [
  'const RUNTIME_SCOPE = process.env.CDP_PORT',
  'Array.isArray(st.browsers)',
  'browsers.find(b => b.profile === want)',
]) {
  assert.ok(cdp.includes(required), `multi-profile CDP contract missing: ${required}`);
}

console.log('multiplayer browser contract: ok');
