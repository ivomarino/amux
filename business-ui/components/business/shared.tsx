'use client';
import {
  Check,
  Clock3,
  TriangleAlert,
  ArrowRight,
  ShieldCheck,
  Inbox,
  CheckCheck,
} from 'lucide-react';
import type { ReactNode } from 'react';
import { Button } from '@/components/ui/button';
import {
  Composer,
  ComposerBar,
  ComposerInput,
  ComposerToolbar,
  ComposerSend,
} from '@/components/assistant-ui/elements/composer';
import { statusOf, type Task } from '@/lib/business';
export function Status({
  task,
  label,
  tone,
}: {
  task?: Task;
  label?: string;
  tone?: string;
}) {
  const s = task
    ? statusOf(task)
    : { label: label || '', tone: tone || 'neutral' };
  const Icon =
    s.tone === 'success'
      ? Check
      : s.tone === 'warning' || s.tone === 'danger'
        ? TriangleAlert
        : s.tone === 'approval'
          ? ShieldCheck
          : Clock3;
  return (
    <span className={'status-badge ' + s.tone}>
      <Icon aria-hidden />
      {s.label}
    </span>
  );
}
export function Empty({
  title,
  description,
  action,
  icon,
}: {
  title: string;
  description: string;
  action?: ReactNode;
  icon?: ReactNode;
}) {
  return (
    <div className="empty">
      <div className="empty-icon">{icon || <Inbox />}</div>
      <h3>{title}</h3>
      <p>{description}</p>
      {action}
    </div>
  );
}
export function SectionLink({
  children,
  onClick,
}: {
  children: ReactNode;
  onClick: () => void;
}) {
  return (
    <button type="button" className="view-all" onClick={onClick}>
      {children}
      <ArrowRight aria-hidden />
    </button>
  );
}
export function Notice({
  children,
  tone = 'warning',
}: {
  children: ReactNode;
  tone?: string;
}) {
  return (
    <div
      className={'notice ' + tone}
      role={tone === 'danger' ? 'alert' : 'status'}
    >
      <TriangleAlert aria-hidden />
      <div>{children}</div>
    </div>
  );
}
export function RequestComposer({
  value,
  setValue,
  onSubmit,
  busy = false,
  disabled = false,
  footer = 'Requests are saved to your work queue.',
  placeholder = 'What would you like Amux to do?',
}: {
  value: string;
  setValue: (v: string) => void;
  onSubmit: () => void;
  busy?: boolean;
  disabled?: boolean;
  footer?: string;
  placeholder?: string;
}) {
  return (
    <div className="composer-wrap">
      <Composer>
        <ComposerBar>
          <ComposerInput
            aria-label="What would you like Amux to do?"
            placeholder={placeholder}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onSubmit={() => {
              if (value.trim() && !busy && !disabled) onSubmit();
            }}
            disabled={busy || disabled}
            maxLength={4000}
          />
          <ComposerToolbar>
            <span className="meta">
              {busy ? 'Saving your request…' : footer}
            </span>
            <ComposerSend
              streaming={false}
              idle={!value.trim() || disabled}
              disabled={busy || disabled || !value.trim()}
              onClick={onSubmit}
              aria-label="Add request to work"
              className="!rounded-lg !bg-blue-600 !text-white disabled:!bg-slate-100 disabled:!text-slate-500"
            />
          </ComposerToolbar>
        </ComposerBar>
      </Composer>
    </div>
  );
}
