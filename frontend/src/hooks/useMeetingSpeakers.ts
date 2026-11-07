'use client';

// Motiv: speakers identified in a meeting (Rust: speakers). Components that
// change speakers dispatch SPEAKERS_CHANGED so the transcript re-reads them.

import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export const SPEAKERS_CHANGED = 'motiv:speakers-changed';

export interface Speaker {
  key: string;
  name: string | null;
  email: string | null;
  profile_id: string | null;
  seconds: number;
}

export interface MeetingSpeakers {
  speakers: Speaker[];
  /** [transcript id, speaker key] */
  assignments: [string, string][];
  analyzed: boolean;
}

export const SPEAKER_COLORS = ['#4F46E5', '#0D9488', '#D97706', '#DB2777', '#2563EB', '#65A30D', '#9333EA', '#DC2626'];

export function speakerLabel(s: Pick<Speaker, 'key' | 'name'>): string {
  return s.name?.trim() || `Speaker ${s.key.replace(/^S/, '')}`;
}

export function speakerColor(key: string): string {
  const n = parseInt(key.replace(/^S/, ''), 10);
  return SPEAKER_COLORS[(Number.isFinite(n) ? n - 1 : 0) % SPEAKER_COLORS.length];
}

export function notifySpeakersChanged(meetingId: string, data?: MeetingSpeakers) {
  window.dispatchEvent(new CustomEvent(SPEAKERS_CHANGED, { detail: { meetingId, data } }));
}

export function useMeetingSpeakers(meetingId: string | undefined) {
  const [data, setData] = useState<MeetingSpeakers | null>(null);

  const reload = useCallback(async () => {
    if (!meetingId) return;
    try {
      setData(await invoke<MeetingSpeakers>('speakers_get', { meetingId }));
    } catch (e) {
      console.error('Failed to load speakers:', e);
    }
  }, [meetingId]);

  useEffect(() => {
    setData(null);
    void reload();
    const onChange = (e: Event) => {
      const d = (e as CustomEvent).detail;
      if (d?.meetingId !== meetingId) return;
      if (d.data) setData(d.data);
      else void reload();
    };
    window.addEventListener(SPEAKERS_CHANGED, onChange);
    return () => window.removeEventListener(SPEAKERS_CHANGED, onChange);
  }, [meetingId, reload]);

  /** transcript id -> { label, color } */
  const byTranscript = useMemo(() => {
    const m = new Map<string, { label: string; color: string; key: string }>();
    if (!data) return m;
    const sp = new Map(data.speakers.map((s) => [s.key, s]));
    for (const [tid, key] of data.assignments) {
      const s = sp.get(key);
      if (s) m.set(tid, { label: speakerLabel(s), color: speakerColor(key), key });
    }
    return m;
  }, [data]);

  return { data, setData, reload, byTranscript };
}
