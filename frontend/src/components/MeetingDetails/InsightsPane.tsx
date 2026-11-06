'use client';

// Motiv: switches the meeting's right-hand pane between summary and action
// items. Both stay mounted so switching keeps editor state. Chat lives in the
// app-wide assistant pane.

import { useState, type ReactNode } from 'react';
import { CheckSquare, Sparkles } from 'lucide-react';

export type InsightsView = 'summary' | 'actions';

interface InsightsPaneProps {
  summary: ReactNode;
  actions: ReactNode;
  initialView?: InsightsView;
}

const VIEWS = [
  { value: 'summary' as const, label: 'Summary', icon: Sparkles },
  { value: 'actions' as const, label: 'Actions', icon: CheckSquare },
];

export function InsightsPane({ summary, actions, initialView = 'summary' }: InsightsPaneProps) {
  const [view, setView] = useState<InsightsView>(initialView);

  return (
    <div className="flex h-full min-h-0 w-full flex-col">
      <div className="flex shrink-0 justify-center border-b border-gray-200 bg-white pt-2" role="tablist" aria-label="Summary or actions">
        {VIEWS.map(({ value, label, icon: Icon }) => (
          <button
            key={value}
            type="button"
            role="tab"
            aria-selected={view === value}
            onClick={() => setView(value)}
            className={`flex items-center gap-1.5 border-b-2 px-4 pb-2 text-sm ${
              view === value
                ? 'border-blue-600 text-blue-600'
                : 'border-transparent text-gray-500 hover:text-gray-900'
            }`}
          >
            <Icon className="h-4 w-4" />
            {label}
          </button>
        ))}
      </div>
      <div className={view === 'summary' ? 'flex flex-1 min-h-0' : 'hidden'}>{summary}</div>
      <div className={view === 'actions' ? 'flex flex-1 min-h-0' : 'hidden'}>{actions}</div>
    </div>
  );
}
