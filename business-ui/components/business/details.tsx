'use client';
import { useEffect, useState } from 'react';
import {
  Check,
  ShieldCheck,
  Clock3,
  ExternalLink,
  FileText,
  Mail,
  CheckCheck,
  ArrowRight,
} from 'lucide-react';
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from '@/components/ui/sheet';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import { Input } from '@/components/ui/input';
import { Checkbox } from '@/components/ui/checkbox';
import { ApprovalCard } from '@/components/assistant-ui/elements/approval-card';
import { ArtifactCard } from '@/components/assistant-ui/elements/artifact-card';
import { Timeline } from '@/components/assistant-ui/elements/timeline';
import {
  api,
  collection,
  safeLink,
  statusOf,
  timeAgo,
  type Task,
  type Approval,
  type Snapshot,
  type Schedule,
} from '@/lib/business';
import { Status, Notice, Empty } from './shared';
import type { WorkflowDefinition } from '@/server/business';
export function TaskDetail({
  task,
  onClose,
  onChanged,
}: {
  task: Task | null;
  onClose: () => void;
  onChanged: () => void;
}) {
  const [detail, setDetail] = useState<any>(null),
    [note, setNote] = useState(''),
    [assignee, setAssignee] = useState(''),
    [error, setError] = useState(''),
    [busy, setBusy] = useState(false),
    [loaded, setLoaded] = useState(false);
  useEffect(() => {
    if (!task) return;
    let cancelled = false;
    setDetail(null);
    setLoaded(false);
    setError('');
    setNote('');
    setAssignee('');
    api('tasks/' + encodeURIComponent(task.id))
      .then((d) => {
        if (!cancelled) {
          setDetail(d);
          setAssignee((d.item?.item || d.item)?.reviewer || '');
          setLoaded(true);
        }
      })
      .catch((e) => {
        if (!cancelled) setError(e.message);
      });
    return () => {
      cancelled = true;
    };
  }, [task?.id]);
  const item: Task = detail?.item?.item || detail?.item || task;
  const artifacts = collection(detail?.artifacts);
  const verifications = collection(detail?.verifications);
  async function save() {
    if (!item || busy) return;
    setBusy(true);
    setError('');
    try {
      await api(
        'tasks/' + encodeURIComponent(item.id),
        { rev: item.rev, note, assignee: assignee || undefined },
        'PATCH',
      );
      setDetail(await api('tasks/' + encodeURIComponent(item.id)));
      setNote('');
      onChanged();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Sheet
      open={!!task}
      onOpenChange={(v) => {
        if (!v) onClose();
      }}
    >
      <SheetContent>
        <SheetHeader>
          <SheetTitle>{item?.title || 'Work details'}</SheetTitle>
          <SheetDescription>
            Context, evidence, and the next step.
          </SheetDescription>
        </SheetHeader>
        {item && (
          <div className="detail-content">
            <div className="flex flex-wrap justify-between gap-2">
              <Status task={item} />
              <span className="meta">Updated {timeAgo(item.updated)}</span>
            </div>
            {error && <Notice tone="danger">{error}</Notice>}
            <section className="detail-section">
              <h3>What happened</h3>
              <p className="evidence-text">
                {item.ask_question ||
                  item.last_result ||
                  item.desc ||
                  item.desc_head ||
                  'No additional context has been recorded.'}
              </p>
              {!loaded && <p className="meta">Loading the full work item…</p>}
            </section>
            <section className="detail-section">
              <h3>Next step</h3>
              <p className="muted">
                {item.next_action ||
                  item.blocked_on ||
                  'Review this request and decide how to proceed.'}
              </p>
              {item.ask_actor && (
                <p className="small muted">
                  Decision requested from {item.ask_actor}
                </p>
              )}
            </section>
            <section className="detail-section element">
              <h3>Documents & evidence</h3>
              {item.evidence && (
                <p className="evidence-text rounded-lg border bg-slate-50 p-4">
                  {item.evidence}
                </p>
              )}
              {artifacts.map((a: any, i: number) => {
                const ref = a.ref || a.uri || a.path || a.url;
                const href = safeLink(ref);
                return (
                  <ArtifactCard
                    key={a.id || i}
                    title={
                      a.title || a.name || String(ref || 'Recorded artifact')
                    }
                    meta={
                      href
                        ? 'Open evidence'
                        : String(a.kind || 'Recorded reference')
                    }
                    {...(href
                      ? {
                          role: 'link',
                          tabIndex: 0,
                          onClick: () =>
                            window.open(href, '_blank', 'noopener,noreferrer'),
                          onKeyDown: (e: React.KeyboardEvent) => {
                            if (e.key === 'Enter')
                              window.open(
                                href,
                                '_blank',
                                'noopener,noreferrer',
                              );
                          },
                        }
                      : { className: '!cursor-default' })}
                  />
                );
              })}
              {!item.evidence && !artifacts.length && (
                <p className="small muted">
                  {detail?.artifacts === null
                    ? 'Evidence could not be loaded.'
                    : 'No evidence has been attached yet.'}
                </p>
              )}
            </section>
            <section className="detail-section">
              <h3>Verification</h3>
              {verifications.length ? (
                verifications.map((v: any, i: number) => (
                  <div className="notice neutral" key={v.id || i}>
                    <ShieldCheck />
                    <div>
                      <strong>
                        {v.verdict === 'passed'
                          ? 'Verification passed'
                          : 'Verification needs attention'}
                      </strong>
                      <p className="small">
                        {v.reason ||
                          'Recorded by the Amux verification service.'}
                      </p>
                    </div>
                  </div>
                ))
              ) : (
                <p className="small muted">
                  {detail?.verifications === null
                    ? 'Verification history is unavailable.'
                    : 'No independent verification is recorded for this work.'}
                </p>
              )}
            </section>
            <section className="detail-section">
              <h3>Review & assign</h3>
              <label htmlFor="reviewer" className="small muted">
                Reviewer name
              </label>
              <Input
                id="reviewer"
                placeholder="Name of the person reviewing"
                value={assignee}
                onChange={(e) => setAssignee(e.target.value)}
              />
              <label htmlFor="operator-note" className="small muted">
                Add a note
              </label>
              <Textarea
                id="operator-note"
                placeholder="Add a decision, context, or instructions for the next step."
                value={note}
                onChange={(e) => setNote(e.target.value)}
              />
              <Button
                onClick={() => void save()}
                disabled={
                  busy ||
                  !loaded ||
                  (!note.trim() && assignee === (item.reviewer || ''))
                }
              >
                {busy ? 'Saving…' : 'Save review note'}
              </Button>
            </section>
            <details>
              <summary className="small muted cursor-pointer">
                Advanced reference
              </summary>
              <p className="advanced mt-3">
                {item.id} · revision {item.rev}
              </p>
            </details>
          </div>
        )}
      </SheetContent>
    </Sheet>
  );
}
export function ApprovalDetail({
  approval,
  onClose,
  onChanged,
}: {
  approval: Approval | null;
  onClose: () => void;
  onChanged: () => void;
}) {
  const [confirmed, setConfirmed] = useState(false),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(''),
    [receipt, setReceipt] = useState<string | null>(null);
  useEffect(() => {
    setConfirmed(false);
    setError('');
    setReceipt(null);
  }, [approval?.id]);
  const p = approval?.preview || approval?.payload || {};
  const body = String(
    approval?.review?.body || p.body || p.message || p.text || '',
  );
  const expired = (approval?.expires_in_s ?? 0) <= 0;
  const incomplete =
    approval?.kind === 'email' && (!p.to || !approval?.review?.complete);
  const title =
    approval?.kind === 'email'
      ? `Send email${p.to ? ' to ' + p.to : ''}`
      : 'Allow a one-time request';
  async function decide(action: 'approve' | 'reject') {
    if (!approval || busy) return;
    setBusy(true);
    setError('');
    try {
      const r = await api(
        `approvals/${approval.kind}/${approval.id}/${action}`,
        { confirmed: true },
      );
      setReceipt(
        action === 'reject'
          ? 'Rejected. This action was not released.'
          : approval.kind === 'email'
            ? 'Amux accepted the send. This does not confirm recipient delivery.'
            : 'Amux recorded the one-time approval.',
      );
      onChanged();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Sheet
      open={!!approval}
      onOpenChange={(v) => {
        if (!v) onClose();
      }}
    >
      <SheetContent>
        <SheetHeader>
          <SheetTitle>{title}</SheetTitle>
          <SheetDescription>
            Review the exact action before you decide.
          </SheetDescription>
        </SheetHeader>
        {approval && (
          <div className="detail-content">
            {receipt ? (
              <>
                <Notice tone="success">{receipt}</Notice>
                <Button onClick={onClose}>Back to approvals</Button>
              </>
            ) : (
              <>
                <div className="flex justify-between gap-2">
                  <Status label="Waiting for your approval" tone="approval" />
                  <span className="meta">
                    {expired
                      ? 'Expired'
                      : `Expires in ${Math.ceil((approval.expires_in_s || 0) / 60)} min`}
                  </span>
                </div>
                <section className="detail-section">
                  <h3>Why your decision is needed</h3>
                  <p className="muted">
                    {p.reason ||
                      'Amux is asking before taking an action outside your workspace.'}
                  </p>
                </section>
                <section className="detail-section">
                  <h3>What Amux will do</h3>
                  {approval.kind === 'email' ? (
                    <div className="panel panel-body space-y-3 text-sm">
                      {[
                        ['From', p.from],
                        ['To', p.to],
                        ['Cc', p.cc],
                        ['Bcc', p.bcc],
                        ['Subject', p.subject],
                      ]
                        .filter(([, v]) => v)
                        .map(([k, v]) => (
                          <div key={k} className="flex gap-4">
                            <span className="w-16 shrink-0 text-slate-500">
                              {k}
                            </span>
                            <span className="break-all">{String(v)}</span>
                          </div>
                        ))}
                      <hr />
                      <p className="evidence-text">
                        {body || 'Message preview is unavailable.'}
                      </p>
                      {approval.review?.includes_signature && (
                        <p className="small muted">
                          Your saved Amux email signature will be appended.
                        </p>
                      )}
                      {!!approval.review?.attachment_count && (
                        <p className="small muted">
                          {approval.review.attachment_count} attachments require
                          a complete preview.
                        </p>
                      )}
                    </div>
                  ) : (
                    <div className="panel panel-body">
                      <p className="muted">
                        {p.target
                          ? 'Recipient: ' + p.target
                          : 'One-time permission request'}
                      </p>
                      <p className="evidence-text mt-3">
                        {body || JSON.stringify(p, null, 2)}
                      </p>
                    </div>
                  )}
                </section>
                <section className="detail-section">
                  <h3>Evidence & boundaries</h3>
                  <ul className="rule-list">
                    <li>
                      <ShieldCheck />
                      This approval releases only the frozen action held by
                      Amux.
                    </li>
                    <li>
                      <Clock3 />
                      An expired or already used approval cannot be released
                      again.
                    </li>
                  </ul>
                  <p className="small muted">
                    No additional business verification is included in this
                    approval record. Review the content yourself.
                  </p>
                </section>
                {(expired || incomplete) && (
                  <Notice>
                    {expired
                      ? 'This approval has expired. Ask for a new proposal.'
                      : 'The server has not supplied a complete, reviewable message and attachment preview. Approval is unavailable here; request a complete proposal before sending.'}
                  </Notice>
                )}
                <label className="flex items-start gap-3 text-sm">
                  <Checkbox
                    checked={confirmed}
                    onCheckedChange={(v) => setConfirmed(!!v)}
                    disabled={busy || expired || incomplete}
                  />
                  <span>I reviewed the action and its recipients.</span>
                </label>
                {error && <Notice tone="danger">{error}</Notice>}
                <div className="element">
                  <ApprovalCard
                    state={busy ? 'running' : 'request'}
                    title="Your decision"
                    subtitle="One action, under existing Amux permissions"
                    command={
                      approval.kind === 'email'
                        ? `Send this message to ${p.to || 'the listed recipients'}`
                        : 'Release this one-time request'
                    }
                    disabled={busy || !confirmed || expired || incomplete}
                    approveLabel={
                      approval.kind === 'email'
                        ? 'Approve and send'
                        : 'Allow once'
                    }
                    onAllowOnce={() => void decide('approve')}
                    onDeny={() => void decide('reject')}
                  />
                </div>
                <p className="meta">
                  To change the content, reject this draft and request a new
                  one. An edited draft needs its own approval.
                </p>
              </>
            )}
          </div>
        )}
      </SheetContent>
    </Sheet>
  );
}
export function WorkflowDetail({
  workflow,
  data,
  onClose,
  onChanged,
  onWork,
}: {
  workflow: WorkflowDefinition | null;
  data: Snapshot | null;
  onClose: () => void;
  onChanged: () => void;
  onWork: (id: string) => void;
}) {
  const [busy, setBusy] = useState<string | null>(null),
    [error, setError] = useState('');
  const schedules = (data?.schedules || []).filter((s) =>
    workflow?.scheduleIds.includes(String(s.id)),
  );
  async function toggle(s: Schedule) {
    setBusy(String(s.id));
    setError('');
    try {
      await api(
        'schedules/' + encodeURIComponent(s.id),
        { enabled: !s.enabled },
        'PATCH',
      );
      onChanged();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(null);
    }
  }
  return (
    <Sheet
      open={!!workflow}
      onOpenChange={(v) => {
        if (!v) onClose();
      }}
    >
      <SheetContent>
        <SheetHeader>
          <SheetTitle>{workflow?.name || 'Automation'}</SheetTitle>
          <SheetDescription>{workflow?.description}</SheetDescription>
        </SheetHeader>
        {workflow && (
          <div className="detail-content">
            <section className="detail-section">
              <h3>Successful outcome</h3>
              <p className="muted">
                {workflow.success || 'No success criteria specified.'}
              </p>
            </section>
            <section className="detail-section">
              <h3>Business instructions</h3>
              <p className="evidence-text">
                {workflow.instructions ||
                  'No additional instructions. Existing Amux policies still apply.'}
              </p>
              <p className="meta">
                Included with new work created through this automation.
              </p>
            </section>
            <section className="detail-section">
              <h3>Schedule</h3>
              {schedules.length ? (
                schedules.map((s) => (
                  <div className="panel panel-body" key={s.id}>
                    <div className="flex items-start justify-between gap-3">
                      <div>
                        <h3>{s.title}</h3>
                        <p className="small muted mt-2">
                          {s.schedule_expr || 'Scheduled in Amux'}
                        </p>
                        <p className="meta mt-1">
                          {s.next_run
                            ? 'Next: ' + s.next_run
                            : 'Next run not recorded'}
                        </p>
                      </div>
                      <Status
                        label={s.enabled ? 'Enabled' : 'Paused'}
                        tone={s.enabled ? 'success' : 'neutral'}
                      />
                    </div>
                    <Button
                      className="mt-4"
                      variant="outline"
                      onClick={() => void toggle(s)}
                      disabled={!!busy}
                    >
                      {busy === String(s.id)
                        ? 'Updating…'
                        : s.enabled
                          ? 'Pause schedule'
                          : 'Resume schedule'}
                    </Button>
                  </div>
                ))
              ) : (
                <p className="muted small">
                  No schedules linked. Work can be added manually.
                </p>
              )}
              <p className="meta">
                Pausing a schedule prevents its next scheduled run. Work already
                in progress continues.
              </p>
            </section>
            {error && <Notice tone="danger">{error}</Notice>}
            <section className="detail-section">
              <h3>Connected apps</h3>
              <div className="flex flex-wrap gap-2">
                {workflow.appIds.length ? (
                  workflow.appIds.map((id) => (
                    <Status
                      key={id}
                      label={data?.apps?.find((a) => a.id === id)?.name || id}
                      tone="neutral"
                    />
                  ))
                ) : (
                  <p className="small muted">No apps linked.</p>
                )}
              </div>
            </section>
            <Notice tone="blue">
              Your existing Amux permission rules remain in control. This
              automation definition does not grant access or certify an outcome.
            </Notice>
            <Button onClick={() => onWork(workflow.id)}>
              View work <ArrowRight />
            </Button>
            <details>
              <summary className="cursor-pointer text-sm text-slate-600">
                Advanced bindings
              </summary>
              <p className="advanced mt-3">{workflow.workers.join(', ')}</p>
            </details>
          </div>
        )}
      </SheetContent>
    </Sheet>
  );
}
