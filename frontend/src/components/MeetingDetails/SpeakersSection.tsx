'use client';

// Motiv: identify speakers in a meeting, name them (suggesting meeting
// participants), merge split speakers, and opt in to voiceprints.

import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { AudioLines, Loader2, RefreshCw } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  MeetingSpeakers, Speaker, notifySpeakersChanged, speakerColor, speakerLabel, useMeetingSpeakers,
} from '@/hooks/useMeetingSpeakers';

export interface Participant { name: string; email?: string | null }

interface Props {
  meetingId: string;
  /** Names from the calendar event, offered as suggestions. */
  participants: Participant[];
}

export function talkShare(s: Speaker, all: Speaker[]): number {
  const total = all.reduce((a, x) => a + x.seconds, 0);
  return total > 0 ? Math.round((100 * s.seconds) / total) : 0;
}

export function SpeakersSection({ meetingId, participants }: Props) {
  const { data, setData } = useMeetingSpeakers(meetingId);
  const [running, setRunning] = useState(false);
  const [stage, setStage] = useState<{ stage: string; percent: number } | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ meeting_id: string; stage: string; percent: number }>('speakers-progress', (e) => {
      if (e.payload.meeting_id === meetingId) setStage({ stage: e.payload.stage, percent: e.payload.percent });
    }).then((u) => { unlisten = u; }).catch(() => {});
    return () => unlisten?.();
  }, [meetingId]);

  const update = (d: MeetingSpeakers) => {
    setData(d);
    notifySpeakersChanged(meetingId, d);
  };

  const analyze = async () => {
    setRunning(true);
    setStage(null);
    try {
      update(await invoke<MeetingSpeakers>('speakers_analyze', { meetingId }));
    } catch (e) {
      toast.error('Could not identify speakers', { description: String(e) });
    } finally {
      setRunning(false);
      setStage(null);
    }
  };

  const stageText = !stage
    ? 'Starting…'
    : stage.stage === 'download' ? `Downloading the speaker model (26 MB)… ${stage.percent}%`
    : stage.stage === 'decode' ? 'Reading the recording…'
    : stage.stage === 'analyze' ? `Listening for voices… ${stage.percent}%`
    : 'Finishing…';

  return (
    <section data-testid="speakers-section">
      <div className="mb-2 flex items-center justify-between">
        <h3 className="font-semibold text-gray-900">Speakers</h3>
        {data?.analyzed && (
          <Button variant="ghost" size="sm" onClick={analyze} disabled={running} aria-label="Identify speakers again">
            {running ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />} Re-run
          </Button>
        )}
      </div>

      {running ? (
        <p className="flex items-center gap-2 text-xs text-gray-600" role="status"><Loader2 className="h-3.5 w-3.5 animate-spin" /> {stageText}</p>
      ) : !data?.analyzed ? (
        <div className="rounded-lg border border-dashed border-gray-300 p-4 text-center">
          <AudioLines className="mx-auto mb-2 h-6 w-6 text-gray-300" />
          <p className="mb-3 text-xs text-gray-600">Work out who said what. Runs on this computer from the recording.</p>
          <Button size="sm" onClick={analyze}>Identify speakers</Button>
        </div>
      ) : (
        <ul className="space-y-3">
          {data.speakers.map((s) => (
            <SpeakerRow key={s.key} meetingId={meetingId} speaker={s} all={data.speakers} participants={participants} onChange={update} />
          ))}
        </ul>
      )}
    </section>
  );
}

function SpeakerRow({ meetingId, speaker, all, participants, onChange }: {
  meetingId: string; speaker: Speaker; all: Speaker[]; participants: Participant[]; onChange: (d: MeetingSpeakers) => void;
}) {
  const [name, setName] = useState(speaker.name ?? '');
  const [remember, setRemember] = useState(Boolean(speaker.profile_id));
  const [saving, setSaving] = useState(false);
  useEffect(() => { setName(speaker.name ?? ''); setRemember(Boolean(speaker.profile_id)); }, [speaker.name, speaker.profile_id]);

  const listId = `participants-${meetingId}-${speaker.key}`;
  const others = all.filter((o) => o.key !== speaker.key);
  const dirty = name.trim() !== (speaker.name ?? '') || remember !== Boolean(speaker.profile_id);
  const emailFor = useMemo(() => {
    const p = participants.find((x) => x.name.toLowerCase() === name.trim().toLowerCase());
    return p?.email ?? null;
  }, [participants, name]);

  const save = async () => {
    setSaving(true);
    try {
      onChange(await invoke<MeetingSpeakers>('speakers_rename', {
        meetingId, key: speaker.key, name: name.trim() || null, email: emailFor, remember: remember && Boolean(name.trim()),
      }));
      if (remember && name.trim()) toast.success(`Motiv will recognise ${name.trim()} in future meetings`);
    } catch (e) {
      toast.error('Could not save', { description: String(e) });
    } finally {
      setSaving(false);
    }
  };

  const merge = async (into: string) => {
    if (!into) return;
    try {
      onChange(await invoke<MeetingSpeakers>('speakers_merge', { meetingId, from: speaker.key, into }));
    } catch (e) {
      toast.error('Could not merge', { description: String(e) });
    }
  };

  return (
    <li className="rounded-lg border border-gray-200 p-3">
      <div className="mb-2 flex items-center gap-2 text-xs text-gray-500">
        <span className="inline-block h-2.5 w-2.5 rounded-full" style={{ backgroundColor: speakerColor(speaker.key) }} />
        <span className="font-medium text-gray-900">{speakerLabel(speaker)}</span>
        <span>· {talkShare(speaker, all)}% of talk</span>
        {speaker.profile_id && <span className="rounded-full bg-indigo-50 px-1.5 py-0.5 text-[10px] text-indigo-700">recognised</span>}
      </div>
      <div className="flex gap-2">
        <input
          className="min-w-0 flex-1 rounded-md border border-gray-300 px-2 py-1 text-sm"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter' && dirty) void save(); }}
          list={listId}
          placeholder={participants.length ? 'Name (pick a participant or type)' : 'Name'}
          aria-label={`Name for ${speakerLabel(speaker)}`}
        />
        <datalist id={listId}>
          {participants.map((p) => <option key={`${p.name}-${p.email ?? ''}`} value={p.name} />)}
        </datalist>
        <Button size="sm" variant={dirty ? 'default' : 'outline'} onClick={save} disabled={!dirty || saving}>Save</Button>
      </div>
      <div className="mt-2 flex flex-wrap items-center justify-between gap-2">
        <label className="flex items-center gap-1.5 text-xs text-gray-600" title="Saves a voiceprint on this computer. Delete it anytime in Settings > Meetings.">
          <input type="checkbox" className="h-3.5 w-3.5 rounded border-gray-300" checked={remember} onChange={(e) => setRemember(e.target.checked)} aria-label={`Recognise ${speakerLabel(speaker)}'s voice`} />
          Recognise this voice in future meetings
        </label>
        {others.length > 0 && (
          <select className="rounded-md border border-gray-200 bg-white px-1.5 py-0.5 text-xs text-gray-600" value="" onChange={(e) => void merge(e.target.value)} aria-label={`Merge ${speakerLabel(speaker)} into`}>
            <option value="">Same person as…</option>
            {others.map((o) => <option key={o.key} value={o.key}>{speakerLabel(o)}</option>)}
          </select>
        )}
      </div>
    </li>
  );
}
