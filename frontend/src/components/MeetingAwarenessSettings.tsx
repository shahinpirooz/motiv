'use client';

// Motiv: settings for Zoom / Teams / Google Meet awareness (Rust: meeting_watch).

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Switch } from '@/components/ui/switch';

export type WatchAction = 'ask' | 'auto' | 'off';

export interface MeetingWatchSettings {
  enabled: boolean;
  on_start: WatchAction;
  on_end: WatchAction;
  apps: { zoom: boolean; teams: boolean; meet: boolean };
}

const START_OPTIONS: Array<{ value: WatchAction; label: string }> = [
  { value: 'ask', label: 'Ask me with a popup' },
  { value: 'auto', label: 'Start recording automatically' },
  { value: 'off', label: 'Do nothing' },
];
const END_OPTIONS: Array<{ value: WatchAction; label: string }> = [
  { value: 'ask', label: 'Ask me with a popup' },
  { value: 'auto', label: 'Stop and save automatically' },
  { value: 'off', label: 'Keep recording' },
];
const APPS: Array<{ key: keyof MeetingWatchSettings['apps']; label: string }> = [
  { key: 'zoom', label: 'Zoom' },
  { key: 'teams', label: 'Microsoft Teams' },
  { key: 'meet', label: 'Google Meet' },
];

export function MeetingAwarenessSettings() {
  const [settings, setSettings] = useState<MeetingWatchSettings | null>(null);

  useEffect(() => {
    invoke<MeetingWatchSettings>('meeting_watch_get_settings')
      .then(setSettings)
      .catch((e) => toast.error('Could not load meeting settings', { description: String(e) }));
  }, []);

  const save = async (next: MeetingWatchSettings) => {
    const prev = settings;
    setSettings(next);
    try {
      await invoke('meeting_watch_save_settings', { settings: next });
    } catch (e) {
      setSettings(prev);
      toast.error('Could not save', { description: String(e) });
    }
  };

  if (!settings) return <div className="h-24 animate-pulse rounded bg-gray-100" />;

  return (
    <div className="space-y-6 rounded-lg border border-gray-200 bg-white p-6" data-testid="meeting-awareness-settings">
      <div className="flex items-start justify-between gap-6">
        <div>
          <h3 className="text-lg font-semibold">Meeting awareness</h3>
          <p className="mt-1 text-sm text-gray-600">
            Notice when a Zoom, Teams or Google Meet call starts and ends. Motiv checks which apps are open and using
            the microphone; nothing leaves your computer.
          </p>
        </div>
        <Switch
          checked={settings.enabled}
          onCheckedChange={(enabled) => save({ ...settings, enabled })}
          aria-label="Meeting awareness"
        />
      </div>

      {settings.enabled && (
        <>
          <Choice
            label="When a call starts"
            value={settings.on_start}
            options={START_OPTIONS}
            onChange={(on_start) => save({ ...settings, on_start })}
          />
          <Choice
            label="When the call ends"
            value={settings.on_end}
            options={END_OPTIONS}
            onChange={(on_end) => save({ ...settings, on_end })}
          />
          <div>
            <p className="mb-2 text-sm font-medium text-gray-900">Watch for</p>
            <div className="flex flex-wrap gap-6">
              {APPS.map(({ key, label }) => (
                <label key={key} className="flex items-center gap-2 text-sm text-gray-700">
                  <input
                    type="checkbox"
                    className="h-4 w-4 rounded border-gray-300"
                    checked={settings.apps[key]}
                    onChange={(e) => save({ ...settings, apps: { ...settings.apps, [key]: e.target.checked } })}
                  />
                  {label}
                </label>
              ))}
            </div>
          </div>
          <p className="text-xs text-gray-500">
            On macOS, call names come from window titles, which need the Screen Recording permission Motiv already
            uses for system audio. Google Meet is detected in Chrome, Edge, Safari, Firefox, Arc and Brave.
          </p>
        </>
      )}
    </div>
  );
}

function Choice({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: WatchAction;
  options: Array<{ value: WatchAction; label: string }>;
  onChange: (v: WatchAction) => void;
}) {
  return (
    <label className="flex items-center justify-between gap-6 text-sm">
      <span className="font-medium text-gray-900">{label}</span>
      <select
        className="rounded-md border border-gray-300 bg-white px-3 py-1.5 text-sm"
        value={value}
        onChange={(e) => onChange(e.target.value as WatchAction)}
        aria-label={label}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}
