// Synthetic transport fixtures only. Never connected to customer accounts.
import http from 'node:http';
import {readFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
const now = () => Math.floor(Date.now() / 1000);
let tasks, workflows, schedules, pending, grants, prefs, requests;
function reset() {
  tasks = [
    {
      id: 'TEST-1',
      title: 'Northwind Supply — payment needs matching',
      status: 'needsyou',
      ask_question:
        'Two invoices have the same amount. Confirm which invoice this payment covers.',
      next_action: 'Review the remittance document and choose an invoice.',
      session: 'receivables',
      created: now() - 3600,
      updated: now() - 720,
      rev: 1,
      desc: 'A $1,240 payment arrived without an invoice reference.',
      evidence: 'Payment reference PAY-104. Amount and currency verified.',
      tags: ['business'],
      reviewer: '',
      log: '',
    },
    {
      id: 'TEST-2',
      title: 'Greenfield Studio — missing tax form',
      status: 'blocked',
      session: 'documents',
      created: now() - 3600,
      updated: now() - 1800,
      rev: 1,
      next_action: 'Request the missing W-9 form.',
      desc: 'The vendor record is missing a tax form.',
    },
    {
      id: 'TEST-3',
      title: 'Harbor Services — invoices reconciled',
      status: 'verified',
      session: 'receivables',
      created: now() - 7200,
      updated: now() - 300,
      closed_at: now() - 300,
      rev: 2,
      desc: 'Invoice and payment identities match.',
      evidence: 'Independent source read confirmed all 8 allocations.',
    },
    {
      id: 'TEST-4',
      title: 'Weekly receivables summary',
      status: 'doing',
      session: 'receivables',
      created: now() - 500,
      updated: now() - 120,
      rev: 1,
      next_action: 'Produce the review report.',
    },
    {
      id: 'TEST-5',
      title: 'Review upcoming renewals',
      status: 'backlog',
      session: 'documents',
      created: now() - 100,
      updated: now() - 100,
      rev: 1,
      desc: 'Gather upcoming renewal dates.',
    },
  ];
  workflows = [
    {
      id: 'wf_ar',
      name: 'Receivables review',
      description:
        'Matches incoming payments and brings uncertain invoices to you.',
      workers: ['receivables'],
      scheduleIds: ['SCHED-1'],
      appIds: ['google-gmail', 'quickbooks'],
      instructions:
        'Do not write off balances. Ask when payer identity is uncertain.',
      success: 'Every invoice accounted for with source evidence.',
      actions: [],
      version: 1,
    },
    {
      id: 'wf_docs',
      name: 'Document collection',
      description:
        'Keeps required client documents complete and ready for review.',
      workers: ['documents'],
      scheduleIds: [],
      appIds: ['google-drive'],
      instructions: 'Request review before customer follow-up.',
      success: 'Required documents collected.',
      actions: [],
      version: 1,
    },
  ];
  schedules = [
    {
      id: 'SCHED-1',
      title: 'Morning receivables review',
      session: 'receivables',
      enabled: true,
      schedule_expr: 'Every weekday at 9:00 AM',
      next_run: '2026-09-10T09:00',
      run_count: 22,
      version: 1,
    },
  ];
  pending = [
    {
      id: 'apr_0123456789abcdef',
      session: 'receivables',
      created: now() - 300,
      expires_in_s: 3300,
      preview: {
        to: 'alex@example.test',
        from: 'office@example.test',
        cc: '',
        subject: 'Payment reference for your invoice',
        body: 'Hello Alex, could you confirm which invoice your recent payment covers? Thank you.',
      },
      review: {
        body: 'Hello Alex, could you confirm which invoice your recent payment covers? Thank you.',
        body_complete: true,
        attachment_count: 0,
        includes_signature: false,
        complete: true,
      },
    },
  ];
  grants = [];
  prefs = {};
  requests = [];
}
reset();
const server = http.createServer(async (req, res) => {
  const u = new URL(req.url, 'http://127.0.0.1');
  let raw = '';
  for await (const c of req) raw += c;
  let b = {};
  try {
    b = raw ? JSON.parse(raw) : {};
  } catch {}
  const send = (x, s = 200) => {
    res.writeHead(s, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(x));
  };
  if (u.pathname === '/test/reset') {
    reset();
    return send({ ok: true });
  }
  if (u.pathname === '/test/requests') return send(requests);
  requests.push({
    path: u.pathname,
    method: req.method,
    body: b,
    approver: req.headers['x-amux-approver'],
  });
  if (u.pathname === '/test/expire') {
    pending[0].expires_in_s = 0;
    return send({ ok: true });
  }
  if (u.pathname === '/test/incomplete') {
    delete pending[0].review;
    return send({ ok: true });
  }
  if (u.pathname === '/health')
    return send({ status: 'ok', commit: 'fixture', build: 'fixture' });
  if (u.pathname === '/api/org')
    return send({ name: 'Northstar Business', id: 'fixture' });
  if (u.pathname === '/api/prefs') {
    if (req.method === 'POST') {
      prefs[b.key] = b.value;
      return send({ ok: true });
    }
    return send({
      'business_ui.workflows.v1': JSON.stringify(workflows),
      ...prefs,
    });
  }
  if (u.pathname === '/api/sessions')
    return send([
      {
        name: 'receivables',
        desc: 'Receivables operation',
        running: true,
        status: 'active',
      },
      {
        name: 'documents',
        desc: 'Document collection operation',
        running: true,
        status: 'idle',
      },
    ]);
  if (u.pathname === '/api/connectors')
    return send({
      connectors: [
        {
          id: 'google-gmail',
          label: 'Gmail',
          category: 'Communication',
          status: 'connected',
          usable: true,
        },
        {
          id: 'quickbooks',
          label: 'QuickBooks',
          category: 'Accounting',
          status: 'expired',
          usable: false,
        },
        {
          id: 'google-drive',
          label: 'Google Drive',
          category: 'Documents',
          status: 'connected',
          usable: true,
        },
      ],
    });
  if (u.pathname.endsWith('/test'))
    return send({ ok: true, note: 'Connection verified by the provider.' });
  if (u.pathname === '/api/schedules') return send(schedules);
  if (u.pathname === '/api/schedules/SCHED-1') {
    schedules[0].enabled = b.enabled;
    return send({ ok: true });
  }
  if (u.pathname === '/api/email/approvals') return send({ pending });
  if (u.pathname === '/api/grants') return send({ pending: grants });
  if (u.pathname.startsWith('/api/email/approve/')) {
    if (!pending.length) return send({ error: 'Already used' }, 410);
    if (pending[0].expires_in_s <= 0)
      return send({ error: 'This approval has expired.' }, 410);
    pending = [];
    return send({ ok: true, message_id: 'message-fixture' });
  }
  if (u.pathname.startsWith('/api/email/reject/')) {
    pending = [];
    return send({ ok: true });
  }
  if (u.pathname === '/api/board') {
    if (req.method === 'POST') {
      const t = {
        ...b,
        id: 'TEST-' + (tasks.length + 1),
        created: now(),
        updated: now(),
        rev: 0,
      };
      tasks.push(t);
      return send(t, 201);
    }
    return send(tasks);
  }
  const match = u.pathname.match(/^\/api\/board\/([^/]+)(?:\/(.+))?$/);
  if (match) {
    const t = tasks.find((t) => t.id === match[1]);
    if (!t) return send({ error: 'Not found' }, 404);
    if (match[2] === 'artifacts')
      return send([
        {
          id: 'a1',
          title: 'Remittance reference',
          url: 'https://example.test/evidence',
          kind: 'document',
        },
      ]);
    if (match[2] === 'verifications')
      return send(
        t.status === 'verified' ? [{ id: 'v1', verdict: 'passed' }] : [],
      );
    if (req.method === 'PATCH') {
      if (b.rev !== t.rev)
        return send(
          { error: 'This item changed. Refresh before saving.' },
          409,
        );
      if (b.desc_append) t.desc = (t.desc || '') + b.desc_append;
      if (b.reviewer) t.reviewer = b.reviewer;
      t.rev++;
      return send(t);
    }
    return send(t);
  }
  if(req.method==='GET' && (u.pathname==='/' || u.pathname==='/business/' || u.pathname.startsWith('/business/assets/'))){
    const relative=u.pathname.startsWith('/business/assets/')?u.pathname.slice('/business/'.length):'index.html';
    const file=new URL('../../crates/amux-dashboard/static/business/'+relative,import.meta.url);
    const mime=relative.endsWith('.js')?'text/javascript':relative.endsWith('.css')?'text/css':relative.endsWith('.woff2')?'font/woff2':'text/html';
    res.writeHead(200,{'Content-Type':mime});return res.end(readFileSync(fileURLToPath(file)));
  }
  return send({ error: 'Unimplemented fixture route ' + u.pathname }, 404);
});
server.listen(18824, '127.0.0.1');
