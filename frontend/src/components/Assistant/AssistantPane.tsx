'use client';

// Motiv: app-wide assistant pane on the right side of every screen.
// Inside a meeting it can answer about that meeting or across all meetings;
// everywhere else it answers across all meetings (Rust: chat::global).

import { useCallback, useEffect, useMemo, useState } from 'react';
import { usePathname, useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { FileText, MessageSquare, PanelRightClose } from 'lucide-react';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { ChatThread, type ThreadMessage } from './ChatThread';

export interface Source {
  meeting_id: string;
  title: string;
  date: string;
}

interface AssistantMessage extends ThreadMessage {
  sources?: Source[];
}

type Scope = 'meeting' | 'all';

const OPEN_KEY = 'motiv.assistant.open';
const WIDTH = 380;

const MEETING_SUGGESTIONS = [
  'What were the key decisions?',
  'List the action items and owners.',
  'What questions were left open?',
];
const ALL_SUGGESTIONS = [
  'What did I discuss this week?',
  'What have I committed to recently?',
  'Which meetings mentioned the budget?',
];

function readOpen(): boolean {
  try {
    return localStorage.getItem(OPEN_KEY) !== 'false';
  } catch {
    return true;
  }
}

/** The meeting id in the URL when on a meeting page, else null. */
export function meetingIdFromLocation(pathname: string | null, search: string): string | null {
  if (!pathname?.startsWith('/meeting-details')) return null;
  const id = new URLSearchParams(search).get('id');
  return id && !id.startsWith('intro-call') ? id : null;
}

export function AssistantPane() {
  const pathname = usePathname();
  const router = useRouter();
  const { currentMeeting, setCurrentMeeting } = useSidebar();
  const [open, setOpen] = useState(true);
  const [scope, setScope] = useState<Scope>('meeting');
  const [meetingId, setMeetingId] = useState<string | null>(null);
  const [model, setModel] = useState<{ provider: string | null; model: string | null } | null>(null);

  useEffect(() => { setOpen(readOpen()); }, []);
  useEffect(() => {
    try { localStorage.setItem(OPEN_KEY, String(open)); } catch { /* ignore */ }
  }, [open]);

  // Track the meeting in view; default back to "this meeting" when it changes.
  useEffect(() => {
    const id = meetingIdFromLocation(pathname, typeof window === 'undefined' ? '' : window.location.search);
    setMeetingId(id);
    setScope('meeting');
  }, [pathname, currentMeeting?.id]);

  // Show which model answers; refresh when the pane opens.
  useEffect(() => {
    if (!open) return;
    invoke<any>('api_get_model_config')
      .then((c) => setModel(c ? { provider: c.provider ?? null, model: c.model ?? null } : null))
      .catch(() => setModel(null));
  }, [open, pathname]);

  // ⌘J / Ctrl+J toggles the pane.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'j') {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const effective: Scope = meetingId && scope === 'meeting' ? 'meeting' : 'all';
  const meetingTitle = meetingId && currentMeeting?.id === meetingId ? currentMeeting.title : 'This meeting';
  const hasModel = Boolean(model?.provider && model?.model);
  const disabledReason = model && !hasModel ? 'Choose a summary model in Settings first.' : null;

  const openMeeting = useCallback((s: Source) => {
    setCurrentMeeting({ id: s.meeting_id, title: s.title });
    router.push(`/meeting-details?id=${s.meeting_id}`);
  }, [router, setCurrentMeeting]);

  const handlers = useMemo(() => {
    if (effective === 'meeting' && meetingId) {
      return {
        key: `meeting:${meetingId}`,
        load: () => invoke<AssistantMessage[]>('api_get_meeting_chat', { meetingId }),
        ask: (question: string) => invoke<AssistantMessage[]>('api_chat_with_meeting', { meetingId, question }),
        clear: () => invoke<void>('api_clear_meeting_chat', { meetingId }),
        suggestions: MEETING_SUGGESTIONS,
        emptyText: 'Ask anything about this meeting.',
        placeholder: 'Ask about this meeting…',
      };
    }
    return {
      key: 'all',
      load: () => invoke<AssistantMessage[]>('api_get_assistant_chat'),
      ask: (question: string) => invoke<AssistantMessage[]>('api_chat_all_meetings', { question }),
      clear: () => invoke<void>('api_clear_assistant_chat'),
      suggestions: ALL_SUGGESTIONS,
      emptyText: 'Ask anything across all your meetings.',
      placeholder: 'Ask across all meetings…',
    };
  }, [effective, meetingId]);

  if (!open) {
    return (
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-label="Open assistant"
        title="Assistant (⌘J)"
        className="fixed bottom-6 right-6 z-40 flex h-12 w-12 items-center justify-center rounded-full bg-indigo-600 text-white shadow-lg hover:bg-indigo-700"
      >
        <MessageSquare className="h-5 w-5" />
      </button>
    );
  }

  return (
    <aside
      className="sticky top-0 flex h-screen shrink-0 flex-col border-l border-gray-200 bg-white"
      style={{ width: WIDTH }}
      aria-label="Assistant"
      data-testid="assistant-pane"
    >
      <div className="flex items-center justify-between gap-2 border-b border-gray-200 px-4 py-3">
        <div className="min-w-0">
          <h2 className="text-sm font-semibold text-gray-900">Assistant</h2>
          <p className="truncate text-xs text-gray-500">{hasModel ? `Using ${model?.model}` : ' '}</p>
        </div>
        <button
          type="button"
          onClick={() => setOpen(false)}
          aria-label="Close assistant"
          title="Close (⌘J)"
          className="rounded-md p-1.5 text-gray-500 hover:bg-gray-100 hover:text-gray-900"
        >
          <PanelRightClose className="h-4 w-4" />
        </button>
      </div>

      {meetingId && (
        <div className="px-4 pt-3">
          <div className="grid grid-cols-2 rounded-lg bg-gray-100 p-0.5 text-xs" role="tablist" aria-label="Chat scope">
            {([['meeting', meetingTitle], ['all', 'All meetings']] as const).map(([value, label]) => (
              <button
                key={value}
                type="button"
                role="tab"
                aria-selected={effective === value}
                onClick={() => setScope(value)}
                className={`truncate rounded-md px-2 py-1.5 ${effective === value ? 'bg-white font-medium text-gray-900 shadow-sm' : 'text-gray-500 hover:text-gray-900'}`}
              >
                {label}
              </button>
            ))}
          </div>
        </div>
      )}

      <div className="flex-1 min-h-0">
        <ChatThread<AssistantMessage>
          threadKey={handlers.key}
          load={handlers.load}
          ask={handlers.ask}
          clear={handlers.clear}
          suggestions={handlers.suggestions}
          emptyText={handlers.emptyText}
          placeholder={handlers.placeholder}
          disabledReason={disabledReason}
          renderFooter={(m) =>
            m.sources && m.sources.length > 0 ? (
              <div className="mt-2 flex flex-wrap gap-1.5">
                {m.sources.map((s) => (
                  <button
                    key={s.meeting_id}
                    type="button"
                    onClick={() => openMeeting(s)}
                    title={s.date}
                    className="flex max-w-full items-center gap-1 rounded-full border border-gray-200 bg-gray-50 px-2 py-0.5 text-[11px] text-gray-600 hover:border-indigo-300 hover:text-indigo-700"
                  >
                    <FileText className="h-3 w-3 shrink-0" />
                    <span className="truncate">{s.title}</span>
                  </button>
                ))}
              </div>
            ) : null
          }
        />
      </div>
    </aside>
  );
}
