'use client';
import { useState } from 'react';
import {
  AssistantRuntimeProvider,
  useExternalStoreRuntime,
  ThreadPrimitive,
  MessagePrimitive,
  ActionBarPrimitive,
  type AppendMessage,
  type ThreadMessageLike,
} from '@assistant-ui/react';
import { Copy, ArrowRight, MessageSquare } from 'lucide-react';
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from '@/components/ui/sheet';
import { Button } from '@/components/ui/button';
import { RequestComposer, Notice } from './shared';
import { ArtifactCard } from '@/components/assistant-ui/elements/artifact-card';
import { api, type Task } from '@/lib/business';
export function AssistantPanel({
  open,
  onOpenChange,
  onCreated,
  workflowId,
  onViewWork,
}: {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  onCreated: () => void;
  workflowId?: string;
  onViewWork: (task: Task) => void;
}) {
  const [messages, setMessages] = useState<ThreadMessageLike[]>([]);
  const [draft, setDraft] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [latest, setLatest] = useState<Task | null>(null);
  const submit = async (text: string) => {
    if (busy || !text.trim()) return;
    setBusy(true);
    setError('');
    try {
      const result = await api('tasks', {
        title: text.slice(0, 200),
        description: text,
        workflowId,
      });
      const item = result.item || result;
      setMessages((m) => [
        ...m,
        { role: 'user', content: [{ type: 'text', text }] },
        {
          role: 'assistant',
          content: [
            {
              type: 'text',
              text: 'I added this to Work, ready to plan. Open the work item to review the request and decide what happens next.',
            },
          ],
        },
      ]);
      setLatest(item);
      setDraft('');
      onCreated();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const runtime = useExternalStoreRuntime({
    convertMessage: (message) => message,
    messages,
    isRunning: busy,
    onNew: async (m: AppendMessage) => {
      const text = m.content
        .filter((p) => p.type === 'text')
        .map((p) => p.text)
        .join('\n');
      await submit(text);
    },
  });
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent>
        <SheetHeader>
          <SheetTitle>Ask Amux</SheetTitle>
          <SheetDescription>
            Turn a request into visible work. Nothing gets lost in chat.
          </SheetDescription>
        </SheetHeader>
        <AssistantRuntimeProvider runtime={runtime}>
          <ThreadPrimitive.Root className="assistant-sheet">
            <ThreadPrimitive.Viewport className="chat-scroll">
              {messages.length === 0 && (
                <div className="empty">
                  <div className="empty-icon">
                    <MessageSquare />
                  </div>
                  <h3>What can we take off your list?</h3>
                  <p>
                    Describe the work. Amux will save it to your queue for
                    planning before it runs.
                  </p>
                  <div className="flex flex-wrap justify-center gap-2">
                    {[
                      'Find missing documents',
                      'Prepare a receivables review',
                      'Draft a customer follow-up',
                    ].map((s) => (
                      <Button
                        variant="outline"
                        key={s}
                        onClick={() => setDraft(s)}
                      >
                        {s}
                      </Button>
                    ))}
                  </div>
                </div>
              )}
              <ThreadPrimitive.Messages
                components={{
                  UserMessage: () => (
                    <MessagePrimitive.Root className="chat-message chat-user">
                      <MessagePrimitive.Content />
                    </MessagePrimitive.Root>
                  ),
                  AssistantMessage: () => (
                    <MessagePrimitive.Root className="chat-message chat-assistant">
                      <MessagePrimitive.Content />
                      <ActionBarPrimitive.Root>
                        <ActionBarPrimitive.Copy asChild>
                          <button
                            className="mt-3 flex items-center gap-1 text-xs text-slate-600"
                            aria-label="Copy response"
                          >
                            <Copy size={13} />
                            Copy
                          </button>
                        </ActionBarPrimitive.Copy>
                      </ActionBarPrimitive.Root>
                    </MessagePrimitive.Root>
                  ),
                }}
              />
              {latest && (
                <div className="element">
                  <ArtifactCard
                    title={latest.title || 'Your new work item'}
                    meta="Saved in Work · Ready to plan"
                    role="button"
                    tabIndex={0}
                    onClick={() => onViewWork(latest)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault();
                        onViewWork(latest);
                      }
                    }}
                  />
                </div>
              )}
            </ThreadPrimitive.Viewport>
            {error && <Notice tone="danger">{error}</Notice>}
            <RequestComposer
              value={draft}
              setValue={setDraft}
              busy={busy}
              onSubmit={() => void submit(draft)}
              footer="Saved to Amux. External actions still require approval."
            />
          </ThreadPrimitive.Root>
        </AssistantRuntimeProvider>
      </SheetContent>
    </Sheet>
  );
}
