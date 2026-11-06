'use client';

// Motiv: a generic chat thread (messages, suggestions, composer). Used by the
// app-wide assistant pane for both "this meeting" and "all meetings".

import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { ArrowUp, Loader2, Sparkles, Trash2 } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';

export interface ThreadMessage {
  id: string;
  role: 'user' | 'assistant';
  content: string;
}

interface ChatThreadProps<M extends ThreadMessage> {
  /** Changes when the conversation changes (e.g. meeting id); reloads history. */
  threadKey: string;
  load: () => Promise<M[]>;
  ask: (question: string) => Promise<M[]>;
  clear: () => Promise<void>;
  suggestions: string[];
  emptyText: string;
  placeholder: string;
  /** Why asking is not possible right now (no model, no transcript). */
  disabledReason?: string | null;
  /** Extra content under an assistant message (e.g. source links). */
  renderFooter?: (m: M) => ReactNode;
}

export function ChatThread<M extends ThreadMessage>({
  threadKey, load, ask, clear, suggestions, emptyText, placeholder, disabledReason, renderFooter,
}: ChatThreadProps<M>) {
  const [messages, setMessages] = useState<M[]>([]);
  const [draft, setDraft] = useState('');
  const [pending, setPending] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const bottomRef = useRef<HTMLDivElement>(null);
  const keyRef = useRef(threadKey);
  keyRef.current = threadKey;

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setMessages([]);
    setPending(null);
    load()
      .then((rows) => { if (!cancelled) setMessages(rows); })
      .catch((err) => console.error('Failed to load chat:', err))
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [threadKey]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView?.({ behavior: 'smooth', block: 'end' });
  }, [messages, pending]);

  const send = useCallback(async (text: string) => {
    const question = text.trim();
    if (!question || pending || disabledReason) return;
    const forKey = threadKey;
    setPending(question);
    setDraft('');
    try {
      const saved = await ask(question);
      if (keyRef.current === forKey) setMessages((prev) => [...prev, ...saved]);
    } catch (err) {
      const message = typeof err === 'string' ? err : (err as Error)?.message ?? 'Unknown error';
      toast.error('Could not answer', { description: message });
      if (keyRef.current === forKey) setDraft(question);
    } finally {
      if (keyRef.current === forKey) setPending(null);
    }
  }, [ask, pending, threadKey, disabledReason]);

  const onClear = useCallback(async () => {
    try {
      await clear();
      setMessages([]);
    } catch (err) {
      toast.error('Could not clear chat', { description: String(err) });
    }
  }, [clear]);

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      void send(draft);
    }
  };

  const empty = !loading && messages.length === 0 && !pending;

  return (
    <div className="flex h-full min-h-0 w-full flex-col">
      <div className="flex-1 min-h-0 overflow-y-auto px-4 py-4 space-y-4">
        {empty ? (
          <div className="flex h-full flex-col items-center justify-center px-2 text-center text-gray-500">
            <Sparkles className="mb-3 h-7 w-7 text-indigo-300" />
            <p className="mb-4 text-sm">{disabledReason ?? emptyText}</p>
            {!disabledReason && (
              <div className="flex flex-col items-stretch gap-2 w-full">
                {suggestions.map((s) => (
                  <button
                    key={s}
                    type="button"
                    onClick={() => void send(s)}
                    className="rounded-xl border border-gray-200 px-3 py-2 text-left text-xs text-gray-700 hover:bg-gray-50"
                  >
                    {s}
                  </button>
                ))}
              </div>
            )}
          </div>
        ) : (
          <>
            {messages.map((m) => (
              <Bubble key={m.id} role={m.role} content={m.content} footer={m.role === 'assistant' ? renderFooter?.(m) : null} />
            ))}
            {pending && (
              <>
                <Bubble role="user" content={pending} />
                <div className="flex items-center gap-2 text-sm text-gray-500" role="status">
                  <Loader2 className="h-4 w-4 animate-spin" /> Thinking…
                </div>
              </>
            )}
          </>
        )}
        <div ref={bottomRef} />
      </div>

      <div className="border-t border-gray-200 p-3">
        <div className="relative rounded-2xl border border-gray-300 bg-white focus-within:border-indigo-400 focus-within:ring-2 focus-within:ring-indigo-100">
          <Textarea
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder={disabledReason ?? placeholder}
            disabled={Boolean(disabledReason)}
            rows={2}
            className="min-h-[56px] resize-none border-0 bg-transparent pr-12 text-sm shadow-none focus-visible:ring-0"
            aria-label="Question"
          />
          <Button
            size="icon"
            onClick={() => void send(draft)}
            disabled={Boolean(disabledReason) || Boolean(pending) || !draft.trim()}
            aria-label="Send"
            className="absolute bottom-2 right-2 h-8 w-8 rounded-full bg-indigo-600 hover:bg-indigo-700"
          >
            {pending ? <Loader2 className="h-4 w-4 animate-spin" /> : <ArrowUp className="h-4 w-4" />}
          </Button>
        </div>
        <div className="mt-1.5 flex items-center justify-between px-1">
          <span className="text-[11px] text-gray-400">Enter to send · Shift+Enter for a new line</span>
          {messages.length > 0 && (
            <button type="button" onClick={onClear} disabled={Boolean(pending)} aria-label="Clear chat" className="flex items-center gap-1 text-[11px] text-gray-400 hover:text-gray-700">
              <Trash2 className="h-3 w-3" /> Clear
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

function Bubble({ role, content, footer }: { role: 'user' | 'assistant'; content: string; footer?: ReactNode }) {
  if (role === 'user') {
    return (
      <div className="flex justify-end">
        <div className="max-w-[88%] whitespace-pre-wrap rounded-2xl rounded-br-md bg-indigo-600 px-3.5 py-2 text-sm text-white">{content}</div>
      </div>
    );
  }
  return (
    <div className="flex justify-start">
      <div className="max-w-[92%] text-sm text-gray-900">
        <div className="prose prose-sm max-w-none prose-p:my-1.5 prose-ul:my-1.5 prose-li:my-0.5">
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{content}</ReactMarkdown>
        </div>
        {footer}
      </div>
    </div>
  );
}
