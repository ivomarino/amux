"use client";
// Adapted from assistant-ui Elements approval-card (MIT). Business labels, explicit disable, no standing grant or fabricated exit status.

import type { ComponentProps } from "react";
import { CheckIcon, Loader2Icon, ShieldCheckIcon, XIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { field, inkButton, paper } from "./surfaces";

export type ApprovalState = "request" | "running" | "done" | "denied";

export function ApprovalCard({
  state,
  command,
  title,
  subtitle,
  onAllowOnce,
  disabled = false,
  approveLabel = "Allow once",
  resultLabel = "Action recorded",
  onDeny,
  className,
  ...props
}: Omit<
  ComponentProps<"div">,
  | "children"
  | "state"
  | "command"
  | "title"
  | "subtitle"
  | "onAllowOnce"
  | "onAlwaysAllow"
  | "onDeny"
> & {
  state: ApprovalState;
  command: string;
  title: string;
  subtitle: string;
  onAllowOnce?: () => void;
  disabled?: boolean;
  approveLabel?: string;
  resultLabel?: string;
  onDeny?: () => void;
}) {
  return (
    <div
      data-slot="approval-card"
      className={cn(
        paper,
        "flex w-full max-w-none flex-col gap-3.5 rounded-xl p-4",
        className,
      )}

      {...props}
    >
      <div className="flex items-center gap-3">
        <span className="bg-foreground/[0.05] text-foreground/45 flex size-9 shrink-0 items-center justify-center rounded-xl">
          <ShieldCheckIcon className="size-4" />
        </span>
        <div className="flex flex-col">
          <p className="text-[13.5px] font-medium">{title}</p>
          <p className="text-foreground/45 text-xs">{subtitle}</p>
        </div>
      </div>

      <div
        className={cn(
          field,
          "text-foreground/70 rounded-xl px-3.5 py-2.5 font-sans text-sm",
        )}
      >
        {command}
      </div>

      <div className="flex min-h-11 items-center justify-end gap-2">
        {state === "request" ? (
          <>
            <button
              type="button"
              onClick={onDeny}
              className="text-foreground/55 hover:bg-foreground/[0.06] hover:text-foreground/90 min-h-11 rounded-lg px-3.5 text-xs font-medium transition-[background-color,color,scale] duration-150 active:scale-[0.96]"
            >
              Reject
            </button>

            <button
              type="button"
              onClick={onAllowOnce}
              disabled={disabled}
              className={cn(
                inkButton,
                "disabled:opacity-40 disabled:cursor-not-allowed",
                "flex min-h-11 items-center rounded-lg px-3.5 text-xs font-medium",
              )}
            >
              {approveLabel}
            </button>
          </>
        ) : (
          <div
            key={state}
            className="fade-in animate-in text-foreground/55 flex items-center gap-2 text-xs duration-300"
          >
            {state === "running" ? (
              <>
                <Loader2Icon className="text-foreground/45 size-3.5 animate-spin" />
                Recording your decision…
              </>
            ) : state === "denied" ? (
              <>
                <XIcon className="text-foreground/45 size-3.5" />
                Denied
              </>
            ) : (
              <>
                <CheckIcon className="size-3.5 text-emerald-500" />
                {resultLabel}
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
