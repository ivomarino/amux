'use client';
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from 'react';
import {
  House,
  Inbox,
  ShieldCheck,
  Workflow,
  Blocks,
  Settings,
  LifeBuoy,
  PanelsTopLeft,
  ChevronDown,
  ArrowUpRight,
  ArrowRight,
  Plus,
  RefreshCw,
  Search,
  CheckCheck,
  TriangleAlert,
  Clock3,
  Check,
  Mail,
  FileText,
  Link2,
  CalendarDays,
  MessageSquare,
  Activity,
  ExternalLink,
  BriefcaseBusiness,
  Menu,
  SlidersHorizontal,
} from 'lucide-react';
import {
  SidebarProvider,
  Sidebar,
  SidebarHeader,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarMenu,
  SidebarMenuItem,
  SidebarMenuButton,
  SidebarInset,
} from '@/components/ui/sidebar';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { Skeleton } from '@/components/ui/skeleton';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '@/components/ui/dialog';
import { Toaster, toast } from 'sonner';
import { Timeline } from '@/components/assistant-ui/elements/timeline';
import { AgentStatus } from '@/components/assistant-ui/elements/agent-status';
import {
  scheduleLabel,
  advancedUrl,
  api,
  attention,
  complete,
  scopeTasks,
  timeAgo,
  workflowFor,
  statusOf,
  routes,
  type Snapshot,
  type Task,
  type Approval,
  type View,
  type AppConnection,
} from '@/lib/business';
import type { WorkflowDefinition } from '@/server/business';
import { Status, Empty, Notice, RequestComposer, SectionLink } from './shared';
import { NewWork, NewWorkflow, Choice } from './forms';
import { TaskDetail, ApprovalDetail, WorkflowDetail } from './details';
import { AssistantPanel } from './assistant';
const NAV = [
  { id: 'home', label: 'Home', icon: House },
  { id: 'work', label: 'Work', icon: Inbox },
  { id: 'approvals', label: 'Approvals', icon: ShieldCheck },
  { id: 'automations', label: 'Automations', icon: Workflow },
  { id: 'apps', label: 'Apps', icon: Blocks },
] as const;
export default function BusinessApp() {
  const [data, setData] = useState<Snapshot | null>(null),
    [loading, setLoading] = useState(true),
    [error, setError] = useState(''),
    [view, setView] = useState<View>('home'),
    [scope, setScope] = useState('all'),
    [query, setQuery] = useState(''),
    [filter, setFilter] = useState('attention'),
    [page, setPage] = useState(0);
  const [more, setMore] = useState(false),
    [newWork, setNewWork] = useState(false),
    [newWorkflow, setNewWorkflow] = useState(false),
    [selectedTask, setSelectedTask] = useState<Task | null>(null),
    [selectedApproval, setSelectedApproval] = useState<Approval | null>(null),
    [selectedWorkflow, setSelectedWorkflow] = useState<string | null>(null),
    [assistant, setAssistant] = useState(false),
    [draft, setDraft] = useState(''),
    [saving, setSaving] = useState(false),
    [appDialog, setAppDialog] = useState<AppConnection | null>(null),
    [appBusy, setAppBusy] = useState(false),
    [appResult, setAppResult] = useState(''),
    [appAccount, setAppAccount] = useState('');
  const refreshing = useRef(false);
  const alive = useRef(true);
  const refresh = useCallback(async () => {
    if (refreshing.current) return;
    refreshing.current = true;
    try {
      const s = await api<Snapshot>('snapshot');
      if (alive.current) {
        setData(s);
        setError('');
      }
    } catch (e) {
      if (alive.current) setError((e as Error).message);
    } finally {
      refreshing.current = false;
      if (alive.current) setLoading(false);
    }
  }, []);
  useEffect(() => {
    alive.current = true;
    void refresh();
    const id = setInterval(() => {
      if (!document.hidden) void refresh();
    }, 15000);
    const focus = () => void refresh();
    window.addEventListener('focus', focus);
    return () => {
      alive.current = false;
      clearInterval(id);
      window.removeEventListener('focus', focus);
    };
  }, [refresh]);
  useEffect(() => {
    const sync = () => {
      const v = location.hash.slice(1) as View;
      if (routes.includes(v)) setView(v);
    };
    sync();
    window.addEventListener('hashchange', sync);
    return () => window.removeEventListener('hashchange', sync);
  }, []);
  function navigate(v: View) {
    setView(v);
    history.pushState(null, '', '#' + v);
    setPage(0);
  }
  const definitions = data?.workflows || [];
  const currentWorkflow = definitions.find((w) => w.id === scope);
  const tasks = scopeTasks(data?.tasks || [], currentWorkflow);
  const needs = tasks.filter(attention);
  const completed = tasks.filter(complete);
  const start = new Date();
  start.setHours(0, 0, 0, 0);
  const today = completed.filter(
    (t) => (t.closed_at || t.entered_state_at || 0) * 1000 >= +start,
  );
  const sessions = currentWorkflow
    ? (data?.sessions || []).filter((s) =>
        currentWorkflow.workers.includes(s.name),
      )
    : data?.sessions || [];
  const approvals: Approval[] = [
    ...(data?.approvals || []).map((a) => ({ ...a, kind: 'email' as const })),
    ...(data?.grants || []).map((a) => ({ ...a, kind: 'grant' as const })),
  ].filter(
    (a) =>
      !currentWorkflow ||
      currentWorkflow.workers.includes(a.session || a.requested_by || ''),
  );
  const live = !!data?.connected && !error;
  const needsApps = (data?.apps || []).filter(
    (a) =>
      !a.usable &&
      !['not_configured', 'not_connected', 'missing', 'unconfigured'].includes(
        a.status,
      ),
  );
  const workflowName = (t: Task) =>
    workflowFor(t, definitions)?.name || 'Workspace work';
  const filtered = tasks
    .filter(
      (t) =>
        (filter === 'all' ||
          (filter === 'attention' && attention(t)) ||
          (filter === 'running' && t.status === 'doing') ||
          (filter === 'completed' && complete(t)) ||
          (filter === 'queued' &&
            ['todo', 'backlog', 'armed'].includes(t.status))) &&
        [t.title, t.desc_head, t.last_result, workflowName(t)]
          .join(' ')
          .toLowerCase()
          .includes(query.toLowerCase()),
    )
    .sort((a, b) => b.updated - a.updated);
  useEffect(() => setPage(0), [filter, query, scope]);
  async function submitRequest() {
    if (!draft.trim() || saving) return;
    setSaving(true);
    try {
      const r = await api('tasks', {
        title: draft.slice(0, 200),
        description: draft,
        workflowId: scope === 'all' ? undefined : scope,
      });
      setDraft('');
      toast.success('Request saved to Work');
      setSelectedTask(r.item || r);
      void refresh();
    } catch (e) {
      toast.error((e as Error).message);
    } finally {
      setSaving(false);
    }
  }
  function openApp(a: AppConnection) {
    setAppDialog(a);
    setAppResult('');
    setAppAccount('');
  }
  async function appAction(action: 'test' | 'connect') {
    if (!appDialog) return;
    setAppBusy(true);
    setAppResult('');
    try {
      const r = await api(`apps/${appDialog.id}/${action}`, {
        account: appAccount,
      });
      if (action === 'connect') {
        const url = r.auth_url || r.url || r.authorize_url;
        if (url && /^https?:\/\//.test(url)) {
          window.location.assign(url);
        } else
          setAppResult(
            r.note ||
              r.error ||
              'Open Amux Advanced to finish connecting this account.',
          );
      } else {
        setAppResult(
          r.ok === false || r.usable === false
            ? 'The connection needs attention. Reconnect the account.'
            : r.note ||
                r.detail ||
                'Connection check completed. The app status has been refreshed.',
        );
        void refresh();
      }
    } catch (e) {
      setAppResult((e as Error).message);
    } finally {
      setAppBusy(false);
    }
  }
  const dateLabel = new Intl.DateTimeFormat('en-US', {
    weekday: 'long',
    month: 'long',
    day: 'numeric',
  }).format(new Date());
  const workButton = (
    <Button onClick={() => setNewWork(true)} disabled={!live}>
      <Plus />
      Create work
    </Button>
  );
  function heading(title: string, subtitle: string, action?: ReactNode) {
    return (
      <div className="page-head">
        <div>
          <h1>{title}</h1>
          <p>{subtitle}</p>
        </div>
        {action}
      </div>
    );
  }
  function workflowCard(w: WorkflowDefinition) {
    const wt = scopeTasks(data?.tasks || [], w);
    const schedules = (data?.schedules || []).filter((s) =>
      w.scheduleIds.includes(String(s.id)),
    );
    const running = (data?.sessions || []).some(
      (s) => w.workers.includes(s.name) && s.running,
    );
    const count = wt.filter(attention).length;
    return (
      <article className="panel workflow-card" key={w.id}>
        <div className="flex items-center justify-between gap-2">
          <div className="workflow-icon">
            <Workflow />
          </div>
          <Status
            label={
              count
                ? 'Needs attention'
                : running
                  ? 'Running'
                  : schedules.some((s) => s.enabled)
                    ? 'Scheduled'
                    : 'Ready'
            }
            tone={count ? 'warning' : running ? 'success' : 'neutral'}
          />
        </div>
        <div>
          <h3>{w.name}</h3>
          <p className="mt-2 line-clamp">
            {w.description || 'A coordinated workflow for your business.'}
          </p>
        </div>
        <div className="flex gap-5 text-sm">
          <span>
            <strong>{wt.filter(complete).length}</strong> completed
          </span>
          <span className={count ? 'text-amber-700' : 'text-slate-500'}>
            <strong>{count}</strong> {count === 1 ? 'exception' : 'exceptions'}
          </span>
        </div>
        <div className="small muted flex gap-2 items-center">
          <CalendarDays size={15} />
          {schedules.length === 1
            ? scheduleLabel(schedules[0].schedule_expr)
            : schedules.length
              ? `${schedules.length} linked schedules`
              : 'Runs when work is assigned'}
        </div>
        <div className="workflow-footer">
          <span className="meta">
            {w.appIds
              .map((id) => data?.apps?.find((a) => a.id === id)?.name || id)
              .join(' · ') || 'No apps linked'}
          </span>
          <SectionLink onClick={() => setSelectedWorkflow(w.id)}>
            Details
          </SectionLink>
        </div>
      </article>
    );
  }
  function attentionRows(limit = 5) {
    return needs.slice(0, limit).map((t) => (
      <button
        className="attention-row"
        key={t.id}
        onClick={() => setSelectedTask(t)}
      >
        <span className={'state-icon ' + statusOf(t).tone}>
          {t.status === 'review' || t.status === 'needsyou' ? (
            <ShieldCheck />
          ) : (
            <TriangleAlert />
          )}
        </span>
        <span className="row-main">
          <span className="row-title line-clamp">{t.title}</span>
          <span className="row-desc line-clamp">
            {t.ask_question ||
              t.last_result ||
              t.blocked_on ||
              t.next_action ||
              'Review the details and decide the next step.'}
          </span>
          <span className="meta">
            {workflowName(t)} · {timeAgo(t.updated)}
          </span>
        </span>
        <ArrowRight size={16} className="shrink-0 text-slate-400" />
      </button>
    ));
  }
  function approvalRows() {
    return approvals.map((a) => {
      const p = a.preview || a.payload || {};
      return (
        <button
          className="attention-row"
          key={a.id}
          onClick={() => setSelectedApproval(a)}
        >
          <span className="state-icon approval">
            {a.kind === 'email' ? <Mail /> : <ShieldCheck />}
          </span>
          <span className="row-main">
            <span className="row-title block">
              {a.kind === 'email'
                ? p.subject || 'Email awaiting review'
                : 'One-time permission request'}
            </span>
            <span className="row-desc block">
              {p.to
                ? 'To: ' + p.to
                : p.target
                  ? 'For: ' + p.target
                  : 'Review the proposed action'}
            </span>
            <span className="meta">
              {(a.expires_in_s || 0) > 0
                ? `Expires in ${Math.ceil((a.expires_in_s || 0) / 60)} minutes`
                : 'Expired'}
            </span>
          </span>
          <span className="view-all">
            Review
            <ArrowRight size={14} />
          </span>
        </button>
      );
    });
  }
  return (
    <SidebarProvider style={{ '--sidebar-width': '232px' } as CSSProperties}>
      <a className="skip-link" href="#main-content">
        Skip to content
      </a>
      <Sidebar className="border-r">
        <SidebarHeader className="px-6 pt-7 pb-4">
          <div className="brand-lockup">
            <span className="brand-mark">
              <PanelsTopLeft size={21} />
            </span>
            amux
          </div>
          <div className="brand-edition">Business</div>
        </SidebarHeader>
        <div className="business-picker">
          <span className="avatar">
            <BriefcaseBusiness size={17} />
          </span>
          <div className="min-w-0 flex-1">
            <p className="small font-semibold truncate">
              {data?.org?.name || 'Your workspace'}
            </p>
            <p className="meta">Business operations</p>
          </div>
        </div>
        <SidebarContent>
          <SidebarGroup className="mt-4">
            <SidebarMenu>
              {NAV.map((n) => (
                <SidebarMenuItem key={n.id}>
                  <SidebarMenuButton
                    aria-label={n.label}
                    isActive={view === n.id}
                    onClick={() => navigate(n.id)}
                    aria-current={view === n.id ? 'page' : undefined}
                  >
                    <n.icon />
                    <span>{n.label}</span>
                    {n.id === 'approvals' && approvals.length > 0 && (
                      <span className="ml-auto count !bg-violet-100 !text-violet-700">
                        {approvals.length}
                      </span>
                    )}
                    {n.id === 'work' && needs.length > 0 && (
                      <span className="ml-auto count">{needs.length}</span>
                    )}
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroup>
          <div className="mx-4 mt-5 rounded-lg border border-blue-100 bg-blue-50/50 p-4">
            <div className="flex gap-2 items-center text-blue-700">
              <ShieldCheck size={16} />
              <span className="small font-medium">You're in control</span>
            </div>
            <p className="small muted mt-2">
              Review actions and evidence before Amux takes the next step.
            </p>
            <button
              className="view-all mt-3"
              onClick={() => navigate('approvals')}
            >
              View approvals <ArrowRight size={14} />
            </button>
          </div>
        </SidebarContent>
        <SidebarFooter>
          <SidebarMenu>
            {[
              { id: 'settings', label: 'Settings', icon: Settings },
              { id: 'support', label: 'Help & support', icon: LifeBuoy },
            ].map((n) => (
              <SidebarMenuItem key={n.id}>
                <SidebarMenuButton
                  onClick={() => navigate(n.id as View)}
                  isActive={view === n.id}
                >
                  <n.icon />
                  {n.label}
                </SidebarMenuButton>
              </SidebarMenuItem>
            ))}
          </SidebarMenu>
          <div className="border-t mt-3 pt-4 flex gap-3 items-center">
            <span className="avatar !rounded-full">O</span>
            <div>
              <p className="small font-medium">Your workspace</p>
              <p className="meta">Amux Business</p>
            </div>
          </div>
        </SidebarFooter>
      </Sidebar>
      <SidebarInset className="!bg-slate-50">
        <header className="topbar">
          <div className="flex items-center gap-2">
            <span className="text-slate-400">
              <House size={16} />
            </span>
            <span className="text-slate-300">/</span>
            <span className="text-sm font-medium">
              {view === 'support'
                ? 'Help & support'
                : view.charAt(0).toUpperCase() + view.slice(1)}
            </span>
          </div>
          <div className="topbar-actions">
            <span className="refresh-caption">
              <span
                className={'status-badge ' + (live ? 'success' : 'warning')}
              >
                {live ? <Check size={12} /> : <Clock3 size={12} />}{' '}
                {loading
                  ? 'Connecting'
                  : live
                    ? 'Live connection'
                    : 'Needs reconnecting'}
              </span>
            </span>
            <Button
              variant="ghost"
              aria-label="Refresh workspace"
              onClick={() => void refresh()}
            >
              <RefreshCw size={16} />
            </Button>
            <Button variant="outline" onClick={() => setAssistant(true)}>
              <MessageSquare size={16} />
              <span>Ask Amux</span>
            </Button>
          </div>
        </header>
        <main id="main-content" className="business-main" tabIndex={-1}>
          {error && (
            <div className="mb-5">
              <Notice tone="danger">
                {error}{' '}
                <button
                  className="underline ml-2"
                  onClick={() => void refresh()}
                >
                  Try again
                </button>
              </Notice>
            </div>
          )}
          {data && Object.keys(data.errors).length > 0 && (
            <div className="mb-5">
              <Notice>
                Some information is unavailable:{' '}
                {Object.keys(data.errors)
                  .map(
                    (k) =>
                      (
                        ({
                          tasks: 'work',
                          sessions: 'operations',
                          emails: 'email approvals',
                          grants: 'permission requests',
                          org: 'business details',
                          health: 'connection',
                        }) as Record<string, string>
                      )[k] || k,
                  )
                  .join(', ')}
                . Counts for these sections are not confirmed.{' '}
                <button className="underline" onClick={() => void refresh()}>
                  Refresh
                </button>
              </Notice>
            </div>
          )}
          {loading ? (
            <>
              <div className="page-head">
                <div>
                  <h1>Today</h1>
                  <p className="muted">Loading your business operations…</p>
                </div>
              </div>
              <div className="grid-kpi">
                {[1, 2, 3, 4].map((i) => (
                  <Skeleton className="h-36 rounded-xl" key={i} />
                ))}
              </div>
              <Skeleton className="h-80 rounded-xl" />
            </>
          ) : (
            <>
              {view === 'home' && (
                <>
                  {heading(
                    'Today',
                    'A clear view of your work and what needs you.',
                    workButton,
                  )}
                  <div className="flex flex-wrap items-center justify-between gap-3 mb-6">
                    <p className="small muted">{dateLabel}</p>
                    <div className="w-full sm:w-64">
                      <Choice
                        label="Workflow scope"
                        value={scope}
                        onChange={setScope}
                        options={[
                          { value: 'all', label: 'All workspace work' },
                          ...definitions.map((w) => ({
                            value: w.id,
                            label: w.name,
                          })),
                        ]}
                      />
                    </div>
                  </div>
                  <div className="home-body">
                    <div className="grid-kpi">
                      {[
                        {
                          label: 'Completed today',
                          value: data?.tasks ? today.length : '—',
                          icon: CheckCheck,
                          caption: data?.coverage.capped
                            ? 'Within loaded work; limited history'
                            : 'Based on recorded completion',
                          style: '',
                        },
                        {
                          label: 'Need attention',
                          value: data?.tasks ? needs.length : '—',
                          icon: TriangleAlert,
                          caption: 'A decision or next step is needed',
                          style: 'kpi-warning',
                        },
                        {
                          label: 'Waiting approval',
                          value:
                            data?.approvals && data?.grants
                              ? approvals.length
                              : '—',
                          icon: ShieldCheck,
                          caption: 'You decide before it proceeds',
                          style: 'kpi-approval',
                        },
                        {
                          label: 'Time saved',
                          value: '—',
                          icon: Clock3,
                          caption: 'Not measured in this workspace',
                          style: '',
                        },
                      ].map((k) => (
                        <article className={'kpi ' + k.style} key={k.label}>
                          <div className="kpi-label">
                            {k.label}
                            <k.icon size={18} className="text-slate-400" />
                          </div>
                          <p className="kpi-value">{k.value}</p>
                          <p className="meta">{k.caption}</p>
                        </article>
                      ))}
                    </div>
                    <div className="dashboard-columns">
                      <div className="panel">
                        <div className="panel-head">
                          <h2 className="flex items-center gap-2">
                            Needs your attention{' '}
                            <span className="count">{needs.length}</span>
                          </h2>
                          <SectionLink
                            onClick={() => {
                              setFilter('attention');
                              navigate('work');
                            }}
                          >
                            View all
                          </SectionLink>
                        </div>
                        {data?.tasks === null ? (
                          <Empty
                            title="Work is unavailable"
                            description="Reconnect to load your work queue. Unavailable data is not an empty queue."
                          />
                        ) : needs.length ? (
                          attentionRows()
                        ) : (
                          <Empty
                            title="Nothing needs your attention"
                            description="Amux will bring exceptions here when a decision or more information is needed."
                            icon={<CheckCheck />}
                          />
                        )}
                      </div>
                      <div className="stack">
                        <div className="panel">
                          <div className="panel-head">
                            <h2 className="flex items-center gap-2">
                              <ShieldCheck
                                size={17}
                                className="text-violet-700"
                              />
                              Your approvals
                            </h2>
                            <SectionLink onClick={() => navigate('approvals')}>
                              View all
                            </SectionLink>
                          </div>
                          {data?.approvals === null || data?.grants === null ? (
                            <Empty
                              title="Approval status unavailable"
                              description="Reconnect to check pending decisions."
                            />
                          ) : approvals.length ? (
                            approvalRows().slice(0, 2)
                          ) : (
                            <Empty
                              title="No approvals waiting"
                              description="Consequential actions appear here when Amux needs your decision."
                              icon={<ShieldCheck />}
                            />
                          )}
                        </div>
                        <div className="panel">
                          <div className="panel-head">
                            <h2>Recent activity</h2>
                            <Activity size={16} className="text-slate-400" />
                          </div>
                          <div className="element p-3">
                            {tasks.length ? (
                              <Timeline
                                events={[...tasks]
                                  .sort((a, b) => b.updated - a.updated)
                                  .slice(0, 3)
                                  .map((t) => ({
                                    id: t.id,
                                    when: 'past' as const,
                                    time: timeAgo(t.updated),
                                    title: t.title,
                                    detail: statusOf(t).label,
                                  }))}
                                visibleCount={3}
                                className="!border-0 !bg-white !p-2"
                              />
                            ) : (
                              <p className="small muted p-4">
                                Activity will appear when work is added or
                                updated.
                              </p>
                            )}
                          </div>
                        </div>
                      </div>
                    </div>
                    <section className="workflows-section">
                      <div className="section-head">
                        <h2>Running automatically</h2>
                        <SectionLink onClick={() => navigate('automations')}>
                          All automations
                        </SectionLink>
                      </div>
                      {definitions.length ? (
                        <div className="workflow-grid">
                          {definitions.slice(0, 3).map(workflowCard)}
                        </div>
                      ) : (
                        <div className="panel panel-body flex flex-wrap justify-between gap-5 items-center">
                          <div className="flex gap-4">
                            <div className="workflow-icon">
                              <Workflow />
                            </div>
                            <div>
                              <h3>Bring your operations together</h3>
                              <p className="small muted mt-1">
                                Create a workflow from your existing Amux
                                operations, schedules, and apps.
                              </p>
                            </div>
                          </div>
                          <Button
                            variant="outline"
                            onClick={() => setNewWorkflow(true)}
                          >
                            Create automation <Plus />
                          </Button>
                        </div>
                      )}
                      <div className="mt-6">
                        <RequestComposer
                          value={draft}
                          setValue={setDraft}
                          busy={saving}
                          disabled={!live}
                          onSubmit={() => void submitRequest()}
                          footer="New requests are saved to Work, ready to plan."
                        />
                      </div>
                      <p className="meta mt-4">
                        Updated {timeAgo(data?.updatedAt)} ·{' '}
                        {scope === 'all'
                          ? 'All workspace work. Define automations to organize it by business outcome.'
                          : currentWorkflow?.name}{' '}
                        {data?.coverage.capped
                          ? '· Showing a limited snapshot; use a workflow scope to narrow the view.'
                          : ''}
                      </p>
                    </section>
                  </div>
                </>
              )}
              {view === 'work' && (
                <>
                  {heading(
                    'Work',
                    'Every request, exception, and next step in one place.',
                    workButton,
                  )}
                  <div className="toolbar">
                    <Tabs
                      value={filter}
                      onValueChange={(v) => setFilter(String(v))}
                    >
                      <TabsList className="!h-11 flex-wrap">
                        {[
                          ['attention', 'Needs attention'],
                          ['all', 'All work'],
                          ['running', 'In progress'],
                          ['queued', 'To plan'],
                          ['completed', 'Completed'],
                        ].map(([v, l]) => (
                          <TabsTrigger
                            key={v}
                            value={v}
                            className="!px-3 !min-h-9"
                          >
                            {l}
                          </TabsTrigger>
                        ))}
                      </TabsList>
                    </Tabs>
                    <div className="search-box">
                      <Search />
                      <Input
                        aria-label="Search work"
                        placeholder="Search work…"
                        value={query}
                        onChange={(e) => setQuery(e.target.value)}
                      />
                    </div>
                  </div>
                  <div className="flex justify-between items-center gap-4 mb-4">
                    <p className="small muted">{filtered.length} work items</p>
                    <div className="w-64">
                      <Choice
                        label="Workflow scope"
                        value={scope}
                        onChange={setScope}
                        options={[
                          { value: 'all', label: 'All workspace work' },
                          ...definitions.map((w) => ({
                            value: w.id,
                            label: w.name,
                          })),
                        ]}
                      />
                    </div>
                  </div>
                  <div className="panel">
                    <div className="queue-head">
                      <span>Work & next step</span>
                      <span>Automation</span>
                      <span>Status</span>
                      <span>Updated</span>
                      <span>Action</span>
                    </div>
                    {filtered.length ? (
                      filtered.slice(page * 20, (page + 1) * 20).map((t) => (
                        <button
                          key={t.id}
                          className="queue-row"
                          onClick={() => setSelectedTask(t)}
                        >
                          <span>
                            <span className="row-title line-clamp">
                              {t.title}
                            </span>
                            <span className="row-desc line-clamp">
                              {t.ask_question ||
                                t.last_result ||
                                t.next_action ||
                                t.desc_head ||
                                'Open to review the details.'}
                            </span>
                          </span>
                          <span className="small muted">{workflowName(t)}</span>
                          <span>
                            <Status task={t} />
                          </span>
                          <span className="meta">{timeAgo(t.updated)}</span>
                          <span className="view-all">
                            Review <ArrowRight size={14} />
                          </span>
                        </button>
                      ))
                    ) : (
                      <Empty
                        title={
                          query ? 'No matching work' : 'Nothing in this view'
                        }
                        description={
                          query
                            ? 'Try another search or choose a different filter.'
                            : 'Work appears here as requests are added and automations progress.'
                        }
                        action={
                          <Button
                            variant="outline"
                            onClick={() => {
                              setFilter('all');
                              setQuery('');
                            }}
                          >
                            View all work
                          </Button>
                        }
                      />
                    )}
                  </div>
                  {filtered.length > 20 && (
                    <div className="flex justify-end items-center gap-4 mt-5">
                      <span className="small muted">
                        Page {page + 1} of {Math.ceil(filtered.length / 20)}
                      </span>
                      <Button
                        variant="outline"
                        disabled={!page}
                        onClick={() => setPage((p) => p - 1)}
                      >
                        Previous
                      </Button>
                      <Button
                        variant="outline"
                        disabled={(page + 1) * 20 >= filtered.length}
                        onClick={() => setPage((p) => p + 1)}
                      >
                        Next
                      </Button>
                    </div>
                  )}
                  <p className="meta mt-4">
                    Snapshot includes up to {data?.coverage.limit || 2000} work
                    items and {data?.coverage.completedLimit || 500} recent
                    completed items. Counts describe this loaded scope.
                  </p>
                </>
              )}
              {view === 'approvals' && (
                <>
                  {heading(
                    'Approvals',
                    'Clear decisions. Exact actions. You stay in control.',
                  )}
                  <div className="notice approval mb-6">
                    <ShieldCheck />
                    <div>
                      <strong>Nothing is approved by opening this page.</strong>
                      <p className="small mt-1">
                        Review the action, recipients, and evidence before you
                        release it. Amux’s existing rules remain in effect.
                      </p>
                    </div>
                  </div>
                  <div className="panel">
                    <div className="panel-head">
                      <h2>Waiting for your decision</h2>
                      <span className="count">{approvals.length}</span>
                    </div>
                    {approvals.length ? (
                      approvalRows()
                    ) : (
                      <Empty
                        title={
                          (data?.approvals === null || data?.grants === null)
                            ? 'Approvals are unavailable'
                            : 'No approvals waiting'
                        }
                        description={
                          (data?.approvals === null || data?.grants === null)
                            ? 'Reconnect to confirm whether any decisions are waiting.'
                            : 'Amux will place consequential actions here when it needs your decision.'
                        }
                        icon={<ShieldCheck />}
                        action={
                          <Button
                            variant="outline"
                            onClick={() => navigate('settings')}
                          >
                            Review approval rules
                          </Button>
                        }
                      />
                    )}
                  </div>
                </>
              )}
              {view === 'automations' && (
                <>
                  {heading(
                    'Automations',
                    'Ongoing business outcomes, built from your existing Amux operations.',
                    <Button
                      onClick={() => setNewWorkflow(true)}
                      disabled={!live}
                    >
                      <Plus />
                      Create automation
                    </Button>,
                  )}
                  {definitions.length ? (
                    <div className="workflow-grid">
                      {definitions.map(workflowCard)}
                    </div>
                  ) : (
                    <div className="panel">
                      <Empty
                        title="Your first automation starts here"
                        description="Group existing operations, schedules, and connected apps around a clear business outcome."
                        icon={<Workflow />}
                        action={
                          <Button onClick={() => setNewWorkflow(true)}>
                            Create automation
                          </Button>
                        }
                      />
                    </div>
                  )}
                  <section className="panel mt-6">
                    <div className="panel-head">
                      <h2>Existing operations</h2>
                      <span className="small muted">
                        {sessions.length} available
                      </span>
                    </div>
                    <div className="panel-body">
                      <p className="small muted">
                        Your Amux server is connected. These operations can be
                        composed into business workflows during setup.
                      </p>
                      <div className="mt-4 flex flex-wrap gap-3">
                        {[
                          {
                            label: 'Running',
                            n: sessions.filter(
                              (s) => s.running && !s.needsAttention,
                            ).length,
                            tone: 'success',
                          },
                          {
                            label: 'Need attention',
                            n: sessions.filter((s) => s.needsAttention).length,
                            tone: 'warning',
                          },
                          {
                            label: 'Not running',
                            n: sessions.filter((s) => !s.running).length,
                            tone: 'neutral',
                          },
                        ].map((s) => (
                          <Status
                            key={s.label}
                            label={`${s.n} ${s.label.toLowerCase()}`}
                            tone={s.tone}
                          />
                        ))}
                      </div>
                    </div>
                  </section>
                </>
              )}
              {view === 'apps' && (
                <>
                  {heading(
                    'Connected apps',
                    'The tools your business uses, connected to Amux.',
                  )}
                  <div className="app-grid">
                    {(data?.apps || []).map((a) => (
                      <article className="panel app-tile" key={a.id}>
                        <div className="flex items-center justify-between gap-2">
                          <div className="workflow-icon">
                            <Blocks />
                          </div>
                          <Status
                            label={
                              a.usable
                                ? 'Connected'
                                : a.status === 'not_configured' ||
                                    a.status === 'unconfigured'
                                  ? 'Not configured'
                                  : 'Needs connection'
                            }
                            tone={a.usable ? 'success' : 'warning'}
                          />
                        </div>
                        <div>
                          <h3>{a.name}</h3>
                          <p className="small muted mt-1">{a.category}</p>
                        </div>
                        <p className="small muted flex-1">
                          {a.usable
                            ? 'Available to authorized Amux operations.'
                            : 'Connect an account to make this app available to your workflows.'}
                        </p>
                        <Button variant="outline" onClick={() => openApp(a)}>
                          {a.usable ? 'Manage connection' : 'Connect account'}
                          <ArrowUpRight />
                        </Button>
                      </article>
                    ))}
                  </div>
                  {data?.apps === null && (
                    <Empty
                      title="Apps could not be loaded"
                      description="Reconnect to see which accounts are available."
                    />
                  )}
                  <p className="meta mt-5">
                    Connected means the account is usable. Each operation’s
                    access is still governed by Amux permissions.
                  </p>
                </>
              )}
              {view === 'settings' && (
                <>
                  {heading(
                    'Settings',
                    'Business preferences and the controls behind your automations.',
                  )}
                  <div className="stack max-w-3xl">
                    <section className="panel">
                      <div className="panel-head">
                        <h2>Approval rules</h2>
                        <ShieldCheck size={18} />
                      </div>
                      <div className="panel-body space-y-4">
                        <p className="muted">
                          The Business app uses your existing Amux approval and
                          permission rules. Creating an automation does not
                          grant new access.
                        </p>
                        <ul className="rule-list">
                          <li>
                            <Check />
                            External email proposals wait in Approvals.
                          </li>
                          <li>
                            <Check />
                            Approvals release the original action once.
                          </li>
                          <li>
                            <Check />
                            Missing or incomplete previews cannot be approved
                            here.
                          </li>
                        </ul>
                        <Button
                          variant="outline"
                          onClick={() => navigate('approvals')}
                        >
                          Open approvals
                        </Button>
                      </div>
                    </section>
                    <section className="panel">
                      <div className="panel-head">
                        <h2>Results & measurement</h2>
                      </div>
                      <div className="panel-body space-y-4">
                        <p className="muted">
                          Completion counts come from recorded work. Time
                          savings and financial impact are not estimated until a
                          business baseline and outcome evidence are available.
                        </p>
                        <p className="small muted">
                          Dates use this device’s timezone:{' '}
                          {Intl.DateTimeFormat().resolvedOptions().timeZone}.
                        </p>
                      </div>
                    </section>
                    <section className="panel">
                      <div className="panel-head">
                        <h2>Advanced</h2>
                        <SlidersHorizontal size={18} />
                      </div>
                      <div className="panel-body space-y-4">
                        <p className="muted">
                          Workers, models, permissions, logs, documents, and
                          runtime configuration stay in the Amux Developer
                          interface.
                        </p>
                        <a
                          className="view-all"
                          href={advancedUrl()}
                          target="_blank"
                          rel="noreferrer"
                        >
                          Open Amux Developer <ExternalLink size={14} />
                        </a>
                        <details>
                          <summary className="small muted cursor-pointer">
                            Connection diagnostics
                          </summary>
                          <p className="advanced mt-3">
                            Build: {data?.health?.build || 'Unavailable'}
                            <br />
                            Source: {data?.health?.commit || 'Unavailable'}
                          </p>
                        </details>
                      </div>
                    </section>
                  </div>
                </>
              )}
              {view === 'support' && (
                <>
                  {heading(
                    'Help & support',
                    'Understand a problem and keep the work moving.',
                  )}
                  <div className="panel max-w-3xl">
                    <div className="panel-body space-y-5">
                      <h3>Check your connection</h3>
                      <p className="muted">
                        If information is stale, refresh the workspace. Your
                        work remains on the Amux server.
                      </p>
                      <Button variant="outline" onClick={() => void refresh()}>
                        <RefreshCw />
                        Refresh workspace
                      </Button>
                      <hr />
                      <h3>Something needs a closer look?</h3>
                      <p className="muted">
                        Create a support work item with what happened and the
                        outcome you expected. Do not include passwords or
                        private account keys.
                      </p>
                      <Button onClick={() => setNewWork(true)}>
                        Create support work
                      </Button>
                      <hr />
                      <h3>Detailed troubleshooting</h3>
                      <p className="muted">
                        Open Settings → Advanced for server logs and runtime
                        controls.
                      </p>
                      <Button
                        variant="outline"
                        onClick={() => navigate('settings')}
                      >
                        Open settings
                      </Button>
                    </div>
                  </div>
                </>
              )}
            </>
          )}
        </main>
        <nav className="bottom-nav" aria-label="Mobile navigation">
          {NAV.slice(0, 4).map((n) => (
            <button
              key={n.id}
              aria-current={view === n.id ? 'page' : undefined}
              onClick={() => navigate(n.id)}
            >
              <n.icon />
              {n.label}
            </button>
          ))}
          <button
            onClick={() => setMore(true)}
            aria-current={
              ['apps', 'settings', 'support'].includes(view)
                ? 'page'
                : undefined
            }
          >
            <Menu />
            More
          </button>
        </nav>
      </SidebarInset>
      <NewWork
        open={newWork}
        onClose={() => setNewWork(false)}
        data={data}
        onCreated={(t) => {
          toast.success('Work saved, ready to plan');
          setSelectedTask(t);
          void refresh();
        }}
      />
      <NewWorkflow
        open={newWorkflow}
        onClose={() => setNewWorkflow(false)}
        data={data}
        onCreated={() => {
          toast.success('Automation saved');
          void refresh();
        }}
      />
      <TaskDetail
        task={selectedTask}
        onClose={() => setSelectedTask(null)}
        onChanged={() => void refresh()}
      />
      <ApprovalDetail
        approval={selectedApproval}
        onClose={() => setSelectedApproval(null)}
        onChanged={() => void refresh()}
      />
      <WorkflowDetail
        workflow={definitions.find((w) => w.id === selectedWorkflow) || null}
        data={data}
        onClose={() => setSelectedWorkflow(null)}
        onChanged={() => void refresh()}
        onWork={(id) => {
          setScope(id);
          setFilter('all');
          navigate('work');
          setSelectedWorkflow(null);
        }}
      />
      <AssistantPanel
        open={assistant}
        onOpenChange={setAssistant}
        onCreated={() => void refresh()}
        workflowId={scope === 'all' ? undefined : scope}
        onViewWork={(t) => {
          setAssistant(false);
          setSelectedTask(t);
        }}
      />
      <Dialog
        open={!!appDialog}
        onOpenChange={(v) => {
          if (!v) setAppDialog(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{appDialog?.name}</DialogTitle>
            <DialogDescription>
              Manage the connection through your existing Amux account.
            </DialogDescription>
          </DialogHeader>
          {appDialog && (
            <div className="form-stack">
              <Status
                label={appDialog.usable ? 'Connected' : 'Needs connection'}
                tone={appDialog.usable ? 'success' : 'warning'}
              />
              <p className="small muted">
                {appDialog.setupNote ||
                  'Use the account you want Amux to access. Existing account permissions apply.'}
              </p>
              {!appDialog.usable && (
                <div className="field-stack">
                  <label htmlFor="app-account">Account email</label>
                  <Input
                    type="email"
                    id="app-account"
                    placeholder="you@yourbusiness.com"
                    value={appAccount}
                    onChange={(e) => setAppAccount(e.target.value)}
                  />
                </div>
              )}
              {appResult && <Notice tone="blue">{appResult}</Notice>}
              <div className="form-actions">
                <Button
                  variant="outline"
                  disabled={appBusy}
                  onClick={() => void appAction('test')}
                >
                  Check connection
                </Button>
                <Button
                  disabled={appBusy}
                  onClick={() => void appAction('connect')}
                >
                  {appBusy
                    ? 'Connecting…'
                    : appDialog.usable
                      ? 'Reconnect account'
                      : 'Continue to sign in'}
                </Button>
              </div>
            </div>
          )}
        </DialogContent>
      </Dialog>
      <Dialog open={more} onOpenChange={setMore}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>More</DialogTitle>
            <DialogDescription>
              Your connected apps and workspace settings.
            </DialogDescription>
          </DialogHeader>
          {(
            [
              { id: 'apps', label: 'Apps' },
              { id: 'settings', label: 'Settings' },
              { id: 'support', label: 'Help & support' },
            ] as const
          ).map((n) => (
            <Button
              key={n.id}
              variant="outline"
              onClick={() => {
                navigate(n.id);
                setMore(false);
              }}
            >
              {n.label}
            </Button>
          ))}
        </DialogContent>
      </Dialog>
      <Toaster position="bottom-right" richColors />
    </SidebarProvider>
  );
}
