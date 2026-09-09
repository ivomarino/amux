/** Business projection and transport adapter. Amux remains the only state/permission/execution authority. */
export type Transport = (path: string, init?: RequestInit) => Promise<Response>;
export type WorkflowDefinition = {
  id: string;
  name: string;
  description: string;
  workers: string[];
  scheduleIds: string[];
  appIds: string[];
  instructions: string;
  success: string;
  actions: string[];
  version: number;
};
const CONFIG_KEY = 'business_ui.workflows.v1';
const json = (value: unknown, status = 200) =>
  Response.json(value, { status, headers: { 'Cache-Control': 'no-store' } });
const object = (x: any) => x && typeof x === 'object' && !Array.isArray(x);
const safeId = (x: string) => /^[a-zA-Z0-9_-]{1,120}$/.test(x);
export class UpstreamError extends Error {
  constructor(
    public status: number,
    public detail: any,
  ) {
    super(
      typeof detail?.error === 'string'
        ? detail.error
        : 'Amux could not complete this request.',
    );
  }
}
export async function handleBusiness(
  request: Request,
  transport: Transport,
): Promise<Response> {
  const path = new URL(request.url).pathname.replace(/^\/api\/business\/?/, '');
  const method = request.method;
  const call = async (p: string, init?: RequestInit): Promise<any> => {
    const response = await transport(p, init);
    const body = await response
      .json()
      .catch(() => ({ error: 'Amux returned an unreadable response.' }));
    // /health deliberately returns 503 for a reachable but degraded server.
    // Preserve that state; it is not an authentication or connection failure.
    if (p === '/health' && response.status === 503 && body !== null &&
        typeof body === 'object' && 'status' in body && body.status === 'degraded') {
      console.warn('[amux-business] verdict=backend_degraded status=503');
      return body;
    }
    if (!response.ok) throw new UpstreamError(response.status, body);
    return body;
  };
  const body = async () => {
    const text = await request.text();
    if (text.length > 100_000)
      throw new UpstreamError(413, { error: 'This request is too large.' });
    try {
      return JSON.parse(text);
    } catch {
      throw new UpstreamError(400, {
        error: 'Please check the information and try again.',
      });
    }
  };
  const write = (
    p: string,
    b: any,
    m = 'POST',
    extra: Record<string, string> = {},
  ) =>
    call(p, {
      method: m,
      headers: { 'Content-Type': 'application/json', ...extra },
      body: JSON.stringify(b),
    });
  const configuration = async () => {
    const prefs = await call('/api/prefs');
    const legacy = prefs[CONFIG_KEY] ? JSON.parse(prefs[CONFIG_KEY]) : [];
    if (!Array.isArray(legacy))
      throw new Error('Workflow configuration is unreadable.');
    const individual = Object.entries(prefs)
      .filter(([key]) => key.startsWith(CONFIG_KEY + '.'))
      .map(([, v]) => JSON.parse(String(v)));
    return [
      ...new Map(
        [...legacy, ...individual].map((w: WorkflowDefinition) => [w.id, w]),
      ).values(),
    ] as WorkflowDefinition[];
  };
  try {
    if (path === 'snapshot' && method === 'GET') {
      const names = [
        'health',
        'org',
        'tasks',
        'sessions',
        'schedules',
        'apps',
        'emails',
        'grants',
        'workflows',
      ];
      const reads = [
        call('/health'),
        call('/api/org'),
        call('/api/board?limit=2000&done_limit=500'),
        call('/api/sessions'),
        call('/api/schedules'),
        call('/api/connectors'),
        call('/api/email/approvals'),
        call('/api/grants'),
        configuration(),
      ];
      const results = await Promise.allSettled(reads);
      const values: any = {};
      const errors: Record<string, string> = {};
      results.forEach((r, i) => {
        if (r.status === 'fulfilled') values[names[i]] = r.value;
        else
          errors[names[i]] =
            r.reason instanceof UpstreamError && r.reason.status === 401
              ? 'Sign in to your Amux server to continue.'
              : 'This information could not be refreshed. Try again.';
      });
      // Expose selected fields to Business views. The embedded transport uses native API permissions; this projection is not an access boundary.
      const sessions = Array.isArray(values.sessions)
        ? values.sessions.map((s: any) => ({
            name: s.name,
            description: s.desc || '',
            running: s.running === true,
            status: s.status || s.state || 'unknown',
            tags: Array.isArray(s.tags)
              ? s.tags
              : typeof s.tags === 'string'
                ? s.tags.split(',')
                : [],
            needsAttention: !!(
              s.api_error ||
              s.credit_limited ||
              s.rate_limited_until > Date.now() / 1000
            ),
            lastActivity: s.last_activity,
          }))
        : null;
      const apps = Array.isArray(values.apps?.connectors)
        ? values.apps.connectors.map((a: any) => ({
            id: a.id,
            name: a.label,
            category: a.category,
            status: a.status,
            usable: a.usable === true,
            detail: a.detail || '',
            setupNote: a.setup_note || '',
            authType: a.auth,
          }))
        : null;
      return json({
        connected: !errors.health,
        updatedAt: new Date().toISOString(),
        org: values.org ? { name: values.org.name, id: values.org.id } : null,
        tasks: values.tasks ?? null,
        sessions,
        schedules: values.schedules ?? null,
        apps,
        approvals: values.emails?.pending ?? null,
        grants: values.grants?.pending ?? null,
        workflows: values.workflows ?? null,
        errors,
        coverage: {
          limit: 2000,
          completedLimit: 500,
          capped: values.tasks?.length >= 2000,
        },
        health: values.health
          ? {
              status: values.health.status,
              commit: values.health.commit,
              build: values.health.build,
            }
          : null,
      });
    }
    if (path === 'workflows' && method === 'POST') {
      const b = await body();
      if (
        typeof b.name !== 'string' ||
        !b.name.trim() ||
        b.name.length > 100 ||
        !Array.isArray(b.workers) ||
        !b.workers.length ||
        !b.workers.every((x: any) => typeof x === 'string' && x.length < 150) ||
        !Array.isArray(b.scheduleIds) ||
        !Array.isArray(b.appIds)
      )
        return json(
          {
            error:
              'Name your automation and choose at least one existing operation.',
          },
          400,
        );
      const [current, sessions, schedules, apps] = await Promise.all([
        configuration(),
        call('/api/sessions'),
        call('/api/schedules'),
        call('/api/connectors'),
      ]);
      if (
        !b.workers.every((n: string) =>
          sessions.some((s: any) => s.name === n),
        ) ||
        !b.scheduleIds.every((n: string) =>
          schedules.some(
            (s: any) => String(s.id) === n && b.workers.includes(s.session),
          ),
        ) ||
        !b.appIds.every((n: string) =>
          apps.connectors.some((a: any) => a.id === n),
        )
      )
        return json(
          {
            error:
              'One of the selected operations, schedules, or apps is no longer available.',
          },
          409,
        );
      if (
        current.some(
          (w) => w.name.toLowerCase() === b.name.trim().toLowerCase(),
        )
      )
        return json(
          { error: 'An automation with this name already exists.' },
          409,
        );
      const w: WorkflowDefinition = {
        id: 'wf_' + crypto.randomUUID().replaceAll('-', ''),
        name: b.name.trim(),
        description: String(b.description || '').slice(0, 600),
        workers: b.workers,
        scheduleIds: b.scheduleIds,
        appIds: b.appIds,
        instructions: String(b.instructions || '').slice(0, 6000),
        success: String(b.success || '').slice(0, 1000),
        actions: [],
        version: 1,
      };
      await write('/api/prefs', {
        key: CONFIG_KEY + '.' + w.id,
        value: JSON.stringify(w),
      });
      return json(w, 201);
    }
    if (path === 'tasks' && method === 'POST') {
      const b = await body();
      if (
        typeof b.title !== 'string' ||
        !b.title.trim() ||
        b.title.length > 200 ||
        typeof b.description !== 'string' ||
        b.description.length > 10_000
      )
        return json(
          { error: 'Add a title (up to 200 characters) and a description.' },
          400,
        );
      const definitions = await configuration();
      const workflow = definitions.find((w) => w.id === b.workflowId);
      if (b.workflowId && !workflow)
        return json(
          { error: 'This automation is no longer available. Choose another.' },
          409,
        );
      // Requests are durable board items. Backlog prevents accidental agent execution from a casual chat submission.
      const result = await write('/api/board', {
        title: b.title.trim(),
        desc: [
          b.description,
          workflow?.instructions &&
            'Business instructions:\n' + workflow.instructions,
          workflow?.success && 'Expected outcome:\n' + workflow.success,
        ]
          .filter(Boolean)
          .join('\n\n'),
        type: 'chore',
        status: 'backlog',
        owner_type: 'human',
        ...(workflow
          ? { session: workflow.workers[0], tags: ['business', workflow.id] }
          : { tags: ['business'] }),
      });
      return json(result, 201);
    }
    const task = path.match(/^tasks\/([A-Za-z0-9_-]+)$/);
    if (task && method === 'GET') {
      const item = await call('/api/board/' + task[1]);
      const extra = await Promise.allSettled([
        call('/api/board/' + task[1] + '/artifacts'),
        call('/api/board/' + task[1] + '/verifications'),
      ]);
      return json({
        item,
        artifacts: extra[0].status === 'fulfilled' ? extra[0].value : null,
        verifications: extra[1].status === 'fulfilled' ? extra[1].value : null,
      });
    }
    if (task && method === 'PATCH') {
      const b = await body();
      if (!Number.isInteger(b.rev))
        return json(
          { error: 'Refresh this work item before changing it.' },
          409,
        );
      const patch: any = { rev: b.rev };
      if (typeof b.note === 'string' && b.note.trim() && b.note.length <= 6000)
        patch.desc_append = '\n\nOperator note:\n' + b.note.trim();
      if (typeof b.assignee === 'string' && b.assignee.length < 120)
        patch.reviewer = b.assignee;
      if (Object.keys(patch).length === 1)
        return json({ error: 'Add a note or choose a reviewer.' }, 400);
      return json(await write('/api/board/' + task[1], patch, 'PATCH'));
    }
    const approval = path.match(
      /^approvals\/(email|grant)\/([A-Za-z0-9_-]+)\/(approve|reject)$/,
    );
    if (approval && method === 'POST') {
      const b = await body();
      if (b.confirmed !== true)
        return json(
          { error: 'Review the full action before confirming.' },
          400,
        );
      if (approval[1] === 'email' && approval[3] === 'approve') {
        const pending = await call('/api/email/approvals');
        const proposal = pending.pending?.find(
          (a: any) => a.id === approval[2],
        );
        if (!proposal)
          return json(
            { error: 'This proposal is no longer pending. Refresh approvals.' },
            409,
          );
        if (!proposal.review?.complete)
          return json(
            {
              error:
                'A complete message and attachment preview is required before approval.',
            },
            409,
          );
      }
      const p =
        approval[1] === 'email'
          ? '/api/email/' + approval[3] + '/' + approval[2]
          : '/api/grants/' + approval[2] + '/' + approval[3];
      // Exactly the existing Amux dashboard's human marker. Does not bypass Amux's approval or identity checks.
      return json(
        await write(p, {}, 'POST', { 'X-Amux-Approver': 'business-ui' }),
      );
    }
    const schedule = path.match(/^schedules\/([A-Za-z0-9_-]+)$/);
    if (schedule && method === 'PATCH') {
      const b = await body();
      if (typeof b.enabled !== 'boolean')
        return json({ error: 'Choose whether this schedule should run.' }, 400);
      return json(
        await write(
          '/api/schedules/' + schedule[1],
          { enabled: b.enabled },
          'PATCH',
        ),
      );
    }
    const app = path.match(/^apps\/([A-Za-z0-9_-]+)\/(test|connect)$/);
    if (app && method === 'POST') {
      const b = await body();
      if (app[2] === 'test')
        return json(await write('/api/connectors/' + app[1] + '/test', {}));
      return json(
        await write('/api/connectors/' + app[1] + '/auth', {
          ...(typeof b.account === 'string' ? { account: b.account } : {}),
        }),
      );
    }
    if (path === 'connection' && method === 'GET')
      return json({
        mode: 'same-server',
        advancedUrl: '/advanced',
        message: 'Connected through the Amux Business gateway.',
      });
    return json(
      { error: 'This operation is not available in Amux Business.' },
      404,
    );
  } catch (e) {
    if (e instanceof UpstreamError)
      return json({ error: e.message, details: e.detail }, e.status);
    console.warn(
      '[business-ui] request_failed',
      method,
      path,
      e instanceof Error ? e.name : 'unknown',
    );
    return json(
      {
        error:
          'Amux could not be reached. Your work is still stored on the server. Reconnect and try again.',
      },
      502,
    );
  }
}
