'use client';
import { useState } from 'react';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';
import { Checkbox } from '@/components/ui/checkbox';
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from '@/components/ui/select';
import { Button } from '@/components/ui/button';
import { api, type Snapshot, type Task } from '@/lib/business';
import { Notice } from './shared';
export function Choice({
  value,
  onChange,
  options,
  label,
  id,
}: {
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
  label: string;
  id?: string;
}) {
  return (
    <Select
      value={value}
      onValueChange={(v) => {
        if (v !== null) onChange(v);
      }}
    >
      <SelectTrigger id={id} aria-label={label} className="min-h-11 w-full">
        <SelectValue>
          {options.find((o) => o.value === value)?.label || label}
        </SelectValue>
      </SelectTrigger>
      <SelectContent>
        {options.map((o) => (
          <SelectItem value={o.value} key={o.value}>
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
export function NewWork({
  open,
  onClose,
  data,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  data: Snapshot | null;
  onCreated: (t: Task) => void;
}) {
  const [title, setTitle] = useState(''),
    [description, setDescription] = useState(''),
    [workflow, setWorkflow] = useState('none'),
    [error, setError] = useState(''),
    [busy, setBusy] = useState(false);
  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (busy) return;
    setBusy(true);
    setError('');
    try {
      const r = await api('tasks', {
        title,
        description,
        workflowId: workflow === 'none' ? undefined : workflow,
      });
      onCreated(r.item || r);
      onClose();
      setTitle('');
      setDescription('');
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog
      open={open}
      onOpenChange={(v) => {
        if (!v && !busy) onClose();
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create work</DialogTitle>
          <DialogDescription>
            Save a request to plan. Creating work does not send a message or
            start an automation.
          </DialogDescription>
        </DialogHeader>
        <form className="form-stack" onSubmit={submit}>
          <div className="field-stack">
            <label htmlFor="work-title">What needs to be done?</label>
            <Input
              id="work-title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Review this month’s outstanding invoices"
              required
              maxLength={200}
            />
          </div>
          <div className="field-stack">
            <label htmlFor="work-description">Instructions and context</label>
            <Textarea
              id="work-description"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="Include the expected outcome and anything Amux should know."
              maxLength={10000}
              className="min-h-28"
            />
          </div>
          <div className="field-stack">
            <label htmlFor="work-workflow">Automation</label>
            <Choice
              id="work-workflow"
              label="Automation for new work"
              value={workflow}
              onChange={setWorkflow}
              options={[
                { value: 'none', label: 'General work' },
                ...(data?.workflows || []).map((w) => ({
                  value: w.id,
                  label: w.name,
                })),
              ]}
            />
          </div>
          {error && <Notice tone="danger">{error}</Notice>}
          <div className="form-actions">
            <Button
              type="button"
              variant="outline"
              onClick={onClose}
              disabled={busy}
            >
              Cancel
            </Button>
            <Button
              disabled={busy || !title.trim() || !data?.connected}
              type="submit"
            >
              {busy ? 'Saving…' : 'Create work'}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
export function NewWorkflow({
  open,
  onClose,
  data,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  data: Snapshot | null;
  onCreated: () => void;
}) {
  const [name, setName] = useState(''),
    [description, setDescription] = useState(''),
    [instructions, setInstructions] = useState(''),
    [success, setSuccess] = useState(''),
    [workers, setWorkers] = useState<string[]>([]),
    [scheduleIds, setSchedules] = useState<string[]>([]),
    [appIds, setApps] = useState<string[]>([]),
    [error, setError] = useState(''),
    [busy, setBusy] = useState(false),
    [step, setStep] = useState(1);
  const toggle = (list: string[], id: string, set: (v: string[]) => void) =>
    set(list.includes(id) ? list.filter((x) => x !== id) : [...list, id]);
  const schedules = (data?.schedules || []).filter((s) =>
    workers.includes(s.session),
  );
  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (step === 1) {
      setStep(2);
      return;
    }
    setBusy(true);
    setError('');
    try {
      await api('workflows', {
        name,
        description,
        instructions,
        success,
        workers,
        scheduleIds: scheduleIds.filter((id) =>
          schedules.some((s) => String(s.id) === id),
        ),
        appIds,
      });
      onCreated();
      onClose();
      setStep(1);
      setName('');
      setWorkers([]);
      setSchedules([]);
      setApps([]);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog
      open={open}
      onOpenChange={(v) => {
        if (!v && !busy) onClose();
      }}
    >
      <DialogContent className="max-h-[90dvh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Create an automation</DialogTitle>
          <DialogDescription>
            Give existing Amux operations one business outcome. Step {step} of
            2.
          </DialogDescription>
        </DialogHeader>
        <form className="form-stack" onSubmit={submit}>
          {step === 1 ? (
            <>
              <div className="field-stack">
                <label htmlFor="automation-name">Automation name</label>
                <Input
                  id="automation-name"
                  placeholder="Receivables review"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  required
                  maxLength={100}
                />
              </div>
              <div className="field-stack">
                <label htmlFor="automation-description">
                  What does it take care of?
                </label>
                <Textarea
                  id="automation-description"
                  placeholder="Reviews open invoices and highlights items that need a decision."
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                />
              </div>
              <div className="field-stack">
                <label htmlFor="automation-success">Successful outcome</label>
                <Input
                  id="automation-success"
                  placeholder="Every invoice accounted for, with evidence"
                  value={success}
                  onChange={(e) => setSuccess(e.target.value)}
                />
              </div>
              <div className="field-stack">
                <label htmlFor="automation-instructions">
                  Business instructions
                </label>
                <Textarea
                  id="automation-instructions"
                  placeholder="Rules to include with new work for this automation."
                  value={instructions}
                  onChange={(e) => setInstructions(e.target.value)}
                />
              </div>
            </>
          ) : (
            <>
              <div className="field-stack">
                <label>Existing operations</label>
                <p className="small muted">
                  Choose the Amux operations that contribute to this outcome. No
                  new runtime is created.
                </p>
                <div className="workflow-form-list">
                  {(data?.sessions || []).map((s) => (
                    <label key={s.name}>
                      <Checkbox
                        checked={workers.includes(s.name)}
                        onCheckedChange={() =>
                          toggle(workers, s.name, setWorkers)
                        }
                      />
                      <span>
                        {s.description || s.name}
                        <span className="meta block">{s.name}</span>
                      </span>
                    </label>
                  ))}
                  {!data?.sessions?.length && (
                    <p className="small muted">
                      No existing operations are available. Set up an operation
                      in Advanced first.
                    </p>
                  )}
                </div>
              </div>
              <div className="field-stack">
                <label>Existing schedules</label>
                <div className="workflow-form-list">
                  {schedules.length ? (
                    schedules.map((s) => (
                      <label key={s.id}>
                        <Checkbox
                          checked={scheduleIds.includes(String(s.id))}
                          onCheckedChange={() =>
                            toggle(scheduleIds, String(s.id), setSchedules)
                          }
                        />
                        {s.title} · {s.enabled ? 'Enabled' : 'Paused'}
                      </label>
                    ))
                  ) : (
                    <p className="small muted">
                      Choose operations above to see their schedules. You can
                      also run work manually.
                    </p>
                  )}
                </div>
              </div>
              <div className="field-stack">
                <label>Connected apps used</label>
                <div className="workflow-form-list">
                  {(data?.apps || []).map((a) => (
                    <label key={a.id}>
                      <Checkbox
                        checked={appIds.includes(a.id)}
                        onCheckedChange={() => toggle(appIds, a.id, setApps)}
                      />
                      {a.name}
                    </label>
                  ))}
                </div>
              </div>
              <Notice tone="blue">
                This saves a workflow definition. Existing permissions and
                schedules stay in effect; it grants no new access.
              </Notice>
            </>
          )}
          {error && <Notice tone="danger">{error}</Notice>}
          <div className="form-actions">
            <Button
              variant="outline"
              type="button"
              disabled={busy}
              onClick={() => (step === 2 ? setStep(1) : onClose())}
            >
              {step === 2 ? 'Back' : 'Cancel'}
            </Button>
            <Button
              type="submit"
              disabled={busy || !name.trim() || (step === 2 && !workers.length)}
            >
              {busy ? 'Saving…' : step === 1 ? 'Continue' : 'Save automation'}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
