'use client';

// Motiv: action items (checklist), decisions, highlights and open questions,
// extracted as structured JSON by the Rust `api_generate_meeting_insights` command.

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { CheckSquare, Copy, Loader2, RefreshCw, Sparkles } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import type { ModelConfig } from '@/components/ModelSettingsModal';

export interface ActionItem {
  task: string;
  owner?: string | null;
  due?: string | null;
  timestamp?: string | null;
  done: boolean;
}

export interface Point {
  text: string;
  timestamp?: string | null;
}

export interface Insights {
  action_items: ActionItem[];
  decisions: Point[];
  highlights: Point[];
  open_questions: Point[];
}

export interface StoredInsights {
  meeting_id: string;
  insights: Insights;
  model: string;
  updated_at: string;
}

interface ActionsPanelProps {
  meetingId: string;
  modelConfig: ModelConfig;
  hasTranscripts: boolean;
}

/** Plain-text/markdown version for pasting into email, Slack or a ticket. */
export function insightsToMarkdown(i: Insights): string {
  const ts = (t?: string | null) => (t ? ` [${t}]` : '');
  const out: string[] = [];
  if (i.action_items.length) {
    out.push('## Action items');
    for (const a of i.action_items) {
      const meta = [a.owner, a.due && `due ${a.due}`].filter(Boolean).join(', ');
      out.push(`- [${a.done ? 'x' : ' '}] ${a.task}${meta ? ` (${meta})` : ''}${ts(a.timestamp)}`);
    }
  }
  for (const [title, list] of [
    ['Decisions', i.decisions],
    ['Highlights', i.highlights],
    ['Open questions', i.open_questions],
  ] as const) {
    if (!list.length) continue;
    out.push('', `## ${title}`);
    for (const p of list) out.push(`- ${p.text}${ts(p.timestamp)}`);
  }
  return out.join('\n').trim();
}

export function ActionsPanel({ meetingId, modelConfig, hasTranscripts }: ActionsPanelProps) {
  const [data, setData] = useState<StoredInsights | null>(null);
  const [loading, setLoading] = useState(true);
  const [generating, setGenerating] = useState(false);
  const activeMeetingRef = useRef(meetingId);
  activeMeetingRef.current = meetingId;
  const hasModel = Boolean(modelConfig?.provider && modelConfig?.model);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setData(null);
    invoke<StoredInsights | null>('api_get_meeting_insights', { meetingId })
      .then((row) => { if (!cancelled) setData(row); })
      .catch((err) => console.error('Failed to load insights:', err))
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [meetingId]);

  const generate = useCallback(async () => {
    const forMeeting = meetingId;
    setGenerating(true);
    try {
      const row = await invoke<StoredInsights>('api_generate_meeting_insights', {
        meetingId,
        model: modelConfig.provider,
        modelName: modelConfig.model,
      });
      if (activeMeetingRef.current === forMeeting) setData(row);
    } catch (err) {
      toast.error('Could not extract action items', { description: String(err) });
    } finally {
      if (activeMeetingRef.current === forMeeting) setGenerating(false);
    }
  }, [meetingId, modelConfig]);

  const toggle = useCallback(async (index: number, done: boolean) => {
    setData((prev) => prev && {
      ...prev,
      insights: {
        ...prev.insights,
        action_items: prev.insights.action_items.map((a, i) => (i === index ? { ...a, done } : a)),
      },
    });
    try {
      await invoke('api_set_action_item_done', { meetingId, index, done });
    } catch (err) {
      toast.error('Could not save', { description: String(err) });
    }
  }, [meetingId]);

  const copy = useCallback(async () => {
    if (!data) return;
    try {
      await navigator.clipboard.writeText(insightsToMarkdown(data.insights));
      toast.success('Copied');
    } catch (err) {
      toast.error('Could not copy', { description: String(err) });
    }
  }, [data]);

  const disabledReason = !hasTranscripts
    ? 'This meeting has no transcript yet.'
    : !hasModel
      ? 'Choose a summary model first (Summary tab, model settings).'
      : null;

  if (loading) return <div className="flex-1" />;

  if (!data) {
    return (
      <div className="flex h-full w-full flex-col items-center justify-center p-6 text-center text-gray-500" data-testid="actions-panel">
        <CheckSquare className="mb-3 h-8 w-8 text-gray-300" />
        <p className="mb-4 text-sm">{disabledReason ?? 'Pull out action items, decisions, highlights and open questions.'}</p>
        <Button onClick={generate} disabled={Boolean(disabledReason) || generating}>
          {generating ? <Loader2 className="h-4 w-4 animate-spin" /> : <Sparkles className="h-4 w-4" />}
          {generating ? 'Extracting…' : 'Extract'}
        </Button>
      </div>
    );
  }

  const { insights } = data;
  const empty = !insights.action_items.length && !insights.decisions.length && !insights.highlights.length && !insights.open_questions.length;

  return (
    <div className="flex h-full min-h-0 w-full flex-col bg-white" data-testid="actions-panel">
      <div className="flex items-center justify-between border-b border-gray-200 px-4 py-3">
        <p className="text-xs text-gray-500">From {data.model}</p>
        <div className="flex gap-1">
          <Button variant="ghost" size="sm" onClick={copy} aria-label="Copy as markdown">
            <Copy className="h-4 w-4" />
          </Button>
          <Button variant="ghost" size="sm" onClick={generate} disabled={Boolean(disabledReason) || generating} aria-label="Regenerate">
            {generating ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
          </Button>
        </div>
      </div>
      <div className="flex-1 min-h-0 overflow-y-auto px-5 py-4 space-y-6 text-sm">
        {empty && <p className="text-gray-500">Nothing found in this meeting.</p>}
        {insights.action_items.length > 0 && (
          <section>
            <h3 className="mb-2 font-semibold text-gray-900">Action items</h3>
            <ul className="space-y-2">
              {insights.action_items.map((a, i) => (
                <li key={i} className="flex items-start gap-2">
                  <input
                    type="checkbox"
                    checked={a.done}
                    onChange={(e) => void toggle(i, e.target.checked)}
                    className="mt-0.5 h-4 w-4 rounded border-gray-300"
                    aria-label={a.task}
                  />
                  <div className={a.done ? 'text-gray-400 line-through' : 'text-gray-900'}>
                    <span>{a.task}</span>
                    {(a.owner || a.due || a.timestamp) && (
                      <span className="ml-2 text-xs text-gray-500">
                        {[a.owner, a.due && `due ${a.due}`, a.timestamp && `[${a.timestamp}]`].filter(Boolean).join(' · ')}
                      </span>
                    )}
                  </div>
                </li>
              ))}
            </ul>
          </section>
        )}
        <PointList title="Decisions" items={insights.decisions} />
        <PointList title="Highlights" items={insights.highlights} />
        <PointList title="Open questions" items={insights.open_questions} />
      </div>
    </div>
  );
}

function PointList({ title, items }: { title: string; items: Point[] }) {
  if (!items.length) return null;
  return (
    <section>
      <h3 className="mb-2 font-semibold text-gray-900">{title}</h3>
      <ul className="list-disc space-y-1 pl-5 text-gray-800">
        {items.map((p, i) => (
          <li key={i}>
            {p.text}
            {p.timestamp && <span className="ml-2 text-xs text-gray-500">[{p.timestamp}]</span>}
          </li>
        ))}
      </ul>
    </section>
  );
}
