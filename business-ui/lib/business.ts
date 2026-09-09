import { handleBusiness } from '@/server/business';
import type { WorkflowDefinition } from '@/server/business';
export type Task = {
  id: string;
  title: string;
  status: string;
  session?: string;
  desc?: string;
  desc_head?: string;
  next_action?: string;
  last_result?: string;
  evidence?: string;
  ask_question?: string;
  ask_actor?: string;
  reviewer?: string;
  blocked_on?: string;
  created: number;
  updated: number;
  closed_at?: number;
  entered_state_at?: number;
  rev: number;
  tags?: string[];
  log?: string;
  source_ref?: string;
};
export type Operation = {
  name: string;
  description: string;
  running: boolean;
  status: string;
  needsAttention: boolean;
  tags: string[];
};
export type Schedule = {
  id: string;
  title: string;
  session: string;
  enabled: boolean;
  next_run?: string;
  last_run?: number;
  schedule_expr?: string;
  run_count: number;
  version?: number;
};
export type AppConnection = {
  id: string;
  name: string;
  status: string;
  usable: boolean;
  category: string;
  detail: string;
  setupNote: string;
  authType: string;
};
export type Approval = {
  id: string;
  session?: string;
  requested_by?: string;
  created?: number;
  expires_in_s?: number;
  review?: {
    body?: string;
    body_complete: boolean;
    attachment_count: number;
    includes_signature: boolean;
    complete: boolean;
  };
  preview?: Record<string, any>;
  payload?: Record<string, any>;
  kind: 'email' | 'grant';
};
export type Snapshot = {
  connected: boolean;
  updatedAt: string;
  org: { name: string; id: string } | null;
  tasks: Task[] | null;
  sessions: Operation[] | null;
  schedules: Schedule[] | null;
  apps: AppConnection[] | null;
  approvals: Approval[] | null;
  grants: Approval[] | null;
  workflows: WorkflowDefinition[] | null;
  errors: Record<string, string>;
  coverage: { limit: number; completedLimit: number; capped: boolean };
  health: { status: string; commit: string; build: string } | null;
};
export const routes = [
  'home',
  'work',
  'approvals',
  'automations',
  'apps',
  'settings',
  'support',
] as const;
export type View = (typeof routes)[number];
export async function api<T = any>(
  path: string,
  body?: unknown,
  method?: string,
): Promise<T> {
  const init: RequestInit = {
    method: method || (body === undefined ? 'GET' : 'POST'),
    headers: { 'Content-Type': 'application/json' },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  };
  const r = (
    import.meta as ImportMeta & { env: { VITE_AMUX_DIRECT?: boolean } }
  ).env.VITE_AMUX_DIRECT
    ? await handleBusiness(
        new Request(location.origin + '/api/business/' + path, init),
        async (p, i) => {
          const headers = new Headers(i?.headers);
          const token = (window as any)._AMUX_AUTH_TOKEN;
          if (token) headers.set('Authorization', 'Bearer ' + token);
          return fetch(p, {
            ...i,
            headers,
            credentials: 'same-origin',
            redirect: 'error',
          });
        },
      )
    : await fetch('/api/business/' + path, init);
  const data: any = await r
    .json()
    .catch(() => ({ error: 'The server returned an unreadable response.' }));
  if (!r.ok)
    throw new Error(
      data.error || 'This action could not be completed. Please try again.',
    );
  return data;
}
export function statusOf(t: Task): { label: string; tone: string } {
  if (t.blocked_on) return { label: 'Needs attention', tone: 'warning' };
  return (
    (
      {
        verified: { label: 'Verified', tone: 'success' },
        done: { label: 'Completed', tone: 'success' },
        doing: { label: 'In progress', tone: 'blue' },
        review: { label: 'Needs review', tone: 'approval' },
        needsyou: { label: 'Needs your decision', tone: 'approval' },
        blocked: { label: 'Needs attention', tone: 'warning' },
        discarded: { label: 'Closed', tone: 'neutral' },
        backlog: { label: 'Ready to plan', tone: 'neutral' },
        todo: { label: 'Queued', tone: 'neutral' },
        armed: { label: 'Scheduled', tone: 'blue' },
      } as Record<string, { label: string; tone: string }>
    )[t.status] || { label: 'Status unavailable', tone: 'neutral' }
  );
}
export function attention(t: Task) {
  return !!t.blocked_on || ['review', 'needsyou', 'blocked'].includes(t.status);
}
export function complete(t: Task) {
  return ['done', 'verified'].includes(t.status);
}
export function timeAgo(v: number | string | undefined) {
  if (!v) return 'Time unavailable';
  const date = typeof v === 'number' ? new Date(v * 1000) : new Date(v);
  if (isNaN(+date)) return 'Time unavailable';
  const minutes = Math.floor((Date.now() - +date) / 60000);
  if (minutes < 1) return 'Just now';
  if (minutes < 60) return `${minutes} min ago`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)} hr ago`;
  return `${Math.floor(minutes / 1440)} days ago`;
}
export function workflowFor(t: Task, ws: WorkflowDefinition[]) {
  return ws.find(
    (w) => t.tags?.includes(w.id) || w.workers.includes(t.session || ''),
  );
}
export function scopeTasks(ts: Task[], w: WorkflowDefinition | undefined) {
  return w
    ? ts.filter(
        (t) => w.workers.includes(t.session || '') || t.tags?.includes(w.id),
      )
    : ts;
}
export function safeLink(ref: unknown): string | undefined {
  if (typeof ref !== 'string') return;
  try {
    const u = new URL(ref);
    if (['https:', 'http:'].includes(u.protocol)) return u.href;
  } catch {}
  return;
}
export function collection(x: any): any[] {
  return Array.isArray(x)
    ? x
    : Array.isArray(x?.items)
      ? x.items
      : Array.isArray(x?.artifacts)
        ? x.artifacts
        : Array.isArray(x?.verifications)
          ? x.verifications
          : [];
}

export const advancedUrl = () =>
  (import.meta as ImportMeta & { env: { VITE_AMUX_DIRECT?: boolean } }).env
    .VITE_AMUX_DIRECT
    ? '/'
    : 'https://localhost:8824/';
