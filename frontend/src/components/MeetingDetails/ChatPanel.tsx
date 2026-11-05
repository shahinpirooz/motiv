'use client';

// Motiv: chat with a meeting. Answers come from the Rust `api_chat_with_meeting`
// command, which uses the summary provider and model configured for the app.

import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Loader2, MessageSquare, Send, Trash2 } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import type { ModelConfig } from '@/components/ModelSettingsModal';

export interface ChatMessage {
  id: string;
  meeting_id: string;
  role: 'user' | 'assistant';
  content: string;
  created_at: string;
}

interface ChatPanelProps {
  meetingId: string;
  modelConfig: ModelConfig;
  hasTranscripts: boolean;
}

const SUGGESTIONS = [
  'What were the key decisions?',
  'List the action items and owners.',
  'What questions were left open?',
];

export function ChatPanel({ meetingId, modelConfig, hasTranscripts }: ChatPanelProps) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [draft, setDraft] = useState('');
  const [pending, setPending] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const bottomRef = useRef<HTMLDivElement>(null);
  const activeMeetingRef = useRef(meetingId);
  activeMeetingRef.current = meetingId;

  const hasModel = Boolean(modelConfig?.provider && modelConfig?.model);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setMessages([]);
    setPending(null);
    invoke<ChatMessage[]>('api_get_meeting_chat', { meetingId })
      .then((rows) => { if (!cancelled) setMessages(rows); })
      .catch((err) => console.error('Failed to load chat:', err))
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [meetingId]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView?.({ behavior: 'smooth', block: 'end' });
  }, [messages, pending]);

  const ask = useCallback(async (text: string) => {
    const question = text.trim();
    if (!question || pending) return;
    const askedFor = meetingId;
    setPending(question);
    setDraft('');
    try {
      const saved = await invoke<ChatMessage[]>('api_chat_with_meeting', {
        meetingId,
        question,
        model: modelConfig.provider,
        modelName: modelConfig.model,
      });
      if (activeMeetingRef.current === askedFor) setMessages((prev) => [...prev, ...saved]);
    } catch (err) {
      const message = typeof err === 'string' ? err : (err as Error)?.message ?? 'Unknown error';
      toast.error('Could not answer', { description: message });
      if (activeMeetingRef.current === askedFor) setDraft(question);
    } finally {
      if (activeMeetingRef.current === askedFor) setPending(null);
    }
  }, [meetingId, modelConfig, pending]);

  const clear = useCallback(async () => {
    try {
      await invoke('api_clear_meeting_chat', { meetingId });
      setMessages([]);
    } catch (err) {
      toast.error('Could not clear chat', { description: String(err) });
    }
  }, [meetingId]);

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      void ask(draft);
    }
  };

  const disabledReason = !hasTranscripts
    ? 'This meeting has no transcript yet.'
    : !hasModel
      ? 'Choose a summary model first (Summary tab, model settings).'
      : null;

  return (
    <div className="flex h-full min-h-0 w-full flex-col bg-white" data-testid="chat-panel">
      <div className="flex items-center justify-between border-b border-gray-200 px-4 py-3">
        <p className="text-xs text-gray-500">
          {hasModel ? `Answers from the transcript using ${modelConfig.model}` : 'No model selected'}
        </p>
        {messages.length > 0 && (
          <Button variant="ghost" size="sm" onClick={clear} disabled={Boolean(pending)} aria-label="Clear chat">
            <Trash2 className="h-4 w-4" />
          </Button>
        )}
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto px-4 py-4 space-y-4">
        {loading ? null : messages.length === 0 && !pending ? (
          <div className="flex h-full flex-col items-center justify-center text-center text-gray-500">
            <MessageSquare className="mb-3 h-8 w-8 text-gray-300" />
            <p className="mb-4 text-sm">{disabledReason ?? 'Ask anything about this meeting.'}</p>
            {!disabledReason && (
              <div className="flex flex-wrap justify-center gap-2">
                {SUGGESTIONS.map((s) => (
                  <button
                    key={s}
                    type="button"
                    onClick={() => void ask(s)}
                    className="rounded-full border border-gray-200 px-3 py-1 text-xs text-gray-700 hover:bg-gray-50"
                  >
                    {s}
                  </button>
                ))}
              </div>
            )}
          </div>
        ) : (
          <>
            {messages.map((m) => <Bubble key={m.id} role={m.role} content={m.content} />)}
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
        <div className="flex items-end gap-2">
          <Textarea
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder={disabledReason ?? 'Ask about this meeting…'}
            disabled={Boolean(disabledReason)}
            rows={2}
            className="min-h-[44px] resize-none text-sm"
            aria-label="Question"
          />
          <Button
            onClick={() => void ask(draft)}
            disabled={Boolean(disabledReason) || Boolean(pending) || !draft.trim()}
            aria-label="Send"
          >
            {pending ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}
          </Button>
        </div>
      </div>
    </div>
  );
}

function Bubble({ role, content }: { role: 'user' | 'assistant'; content: string }) {
  if (role === 'user') {
    return (
      <div className="flex justify-end">
        <div className="max-w-[85%] whitespace-pre-wrap rounded-2xl bg-blue-600 px-4 py-2 text-sm text-white">{content}</div>
      </div>
    );
  }
  return (
    <div className="flex justify-start">
      <div className="prose prose-sm max-w-[85%] rounded-2xl bg-gray-100 px-4 py-2 text-sm text-gray-900">
        <ReactMarkdown remarkPlugins={[remarkGfm]}>{content}</ReactMarkdown>
      </div>
    </div>
  );
}
