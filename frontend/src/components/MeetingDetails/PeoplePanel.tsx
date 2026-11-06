'use client';

// Motiv: the meeting's calendar event, invited attendees and (Teams)
// attendance (Rust: calendar).

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { CalendarDays, Loader2, RefreshCw, Users } from 'lucide-react';
import { Button } from '@/components/ui/button';

export interface Attendee { name: string; email: string; response: string }
export interface Person { name: string; email: string }
export interface CalendarEvent {
  provider: 'microsoft' | 'google';
  id: string;
  title: string;
  start: string;
  end: string;
  all_day: boolean;
  organizer: Person | null;
  attendees: Attendee[];
  join_url: string | null;
  platform: 'teams' | 'zoom' | 'meet' | null;
}
export interface AttendanceRecord { name: string; email: string | null; role: string | null; seconds: number }
export interface MeetingLink { meeting_id: string; event: CalendarEvent; attendance: AttendanceRecord[] | null; source: string }
export interface MeetingCalendar { link: MeetingLink | null; candidates: CalendarEvent[]; connected: boolean }

const PLATFORM_LABEL: Record<string, string> = { teams: 'Teams', zoom: 'Zoom', meet: 'Google Meet' };
const RESPONSE_STYLE: Record<string, string> = {
  accepted: 'bg-green-50 text-green-700',
  tentative: 'bg-amber-50 text-amber-700',
  declined: 'bg-red-50 text-red-700 line-through',
  none: 'bg-gray-100 text-gray-600',
  organizer: 'bg-indigo-50 text-indigo-700',
};

export function formatRange(start: string, end: string): string {
  const s = new Date(start);
  const e = new Date(end);
  const day = s.toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' });
  const t = (d: Date) => d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
  return `${day}, ${t(s)} – ${t(e)}`;
}

export function minutes(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  return `${Math.round(seconds / 60)} min`;
}

export function PeoplePanel({ meetingId }: { meetingId: string }) {
  const [data, setData] = useState<MeetingCalendar | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const active = useRef(meetingId);
  active.current = meetingId;

  const load = useCallback(async () => {
    const forId = meetingId;
    setLoading(true);
    try {
      const d = await invoke<MeetingCalendar>('calendar_for_meeting', { meetingId });
      if (active.current === forId) setData(d);
    } catch (e) {
      toast.error('Could not read your calendar', { description: String(e) });
    } finally {
      if (active.current === forId) setLoading(false);
    }
  }, [meetingId]);

  useEffect(() => { setData(null); void load(); }, [load]);

  const pick = async (eventId: string) => {
    const ev = data?.candidates.find((c) => `${c.provider}:${c.id}` === eventId) ?? null;
    try {
      const link = await invoke<MeetingLink | null>('calendar_set_meeting_event', { meetingId, event: ev });
      setData((d) => d && { ...d, link });
    } catch (e) {
      toast.error('Could not change the event', { description: String(e) });
    }
  };

  const attendance = async () => {
    setBusy(true);
    try {
      const link = await invoke<MeetingLink>('calendar_refresh_attendance', { meetingId });
      setData((d) => d && { ...d, link });
    } catch (e) {
      toast.error('Could not get attendance', { description: String(e) });
    } finally {
      setBusy(false);
    }
  };

  if (loading && !data) {
    return <div className="flex flex-1 items-center justify-center text-sm text-gray-500"><Loader2 className="mr-2 h-4 w-4 animate-spin" /> Checking your calendar…</div>;
  }
  if (!data?.connected) {
    return (
      <div className="flex h-full w-full flex-col items-center justify-center p-6 text-center text-gray-500" data-testid="people-panel">
        <Users className="mb-3 h-8 w-8 text-gray-300" />
        <p className="text-sm">Connect Microsoft 365 or Google in Settings &gt; Accounts to see who was in this meeting.</p>
      </div>
    );
  }

  const link = data.link;
  const ev = link?.event;
  const selected = ev ? `${ev.provider}:${ev.id}` : '';
  const options = [...data.candidates];
  if (ev && !options.some((c) => c.provider === ev.provider && c.id === ev.id)) options.unshift(ev);

  return (
    <div className="flex h-full min-h-0 w-full flex-col bg-white" data-testid="people-panel">
      <div className="space-y-2 border-b border-gray-200 px-5 py-4">
        <div className="flex items-center gap-2 text-xs text-gray-500">
          <CalendarDays className="h-3.5 w-3.5" /> Calendar event
          {link?.source === 'auto' && <span className="text-gray-400">· matched by time</span>}
        </div>
        <select
          className="w-full rounded-md border border-gray-300 bg-white px-3 py-1.5 text-sm"
          value={selected}
          onChange={(e) => void pick(e.target.value)}
          aria-label="Calendar event"
        >
          <option value="">Not linked to an event</option>
          {options.map((c) => (
            <option key={`${c.provider}:${c.id}`} value={`${c.provider}:${c.id}`}>
              {c.title} · {formatRange(c.start, c.end)}
            </option>
          ))}
        </select>
        {ev && (
          <p className="text-xs text-gray-500">
            {formatRange(ev.start, ev.end)}
            {ev.platform && ` · ${PLATFORM_LABEL[ev.platform]}`}
            {ev.organizer && ` · Organised by ${ev.organizer.name || ev.organizer.email}`}
          </p>
        )}
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto px-5 py-4 space-y-6 text-sm">
        {!ev && <p className="text-gray-500">No calendar event overlaps this recording. Pick one above if it belongs to a meeting.</p>}

        {ev && ev.platform === 'teams' && ev.provider === 'microsoft' && (
          <section>
            <div className="mb-2 flex items-center justify-between">
              <h3 className="font-semibold text-gray-900">Attended</h3>
              <Button variant="ghost" size="sm" onClick={attendance} disabled={busy} aria-label="Get attendance">
                {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                {link?.attendance ? 'Refresh' : 'Get attendance'}
              </Button>
            </div>
            {link?.attendance ? (
              <ul className="space-y-1.5">
                {link.attendance.map((a, i) => (
                  <li key={i} className="flex items-center justify-between gap-3">
                    <span className="truncate text-gray-900">{a.name}{a.email && <span className="ml-1.5 text-xs text-gray-500">{a.email}</span>}</span>
                    <span className="shrink-0 text-xs text-gray-500">{a.role ? `${a.role} · ` : ''}{minutes(a.seconds)}</span>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="text-xs text-gray-500">From the Teams attendance report, for meetings you organised.</p>
            )}
          </section>
        )}

        {ev && (
          <section>
            <h3 className="mb-2 font-semibold text-gray-900">Invited ({ev.attendees.length + (ev.organizer ? 1 : 0)})</h3>
            <ul className="space-y-1.5">
              {ev.organizer && <PersonRow name={ev.organizer.name} email={ev.organizer.email} response="organizer" />}
              {ev.attendees.map((a, i) => <PersonRow key={i} name={a.name} email={a.email} response={a.response} />)}
            </ul>
          </section>
        )}
      </div>
    </div>
  );
}

function PersonRow({ name, email, response }: { name: string; email: string; response: string }) {
  return (
    <li className="flex items-center justify-between gap-3">
      <span className="min-w-0 truncate text-gray-900">
        {name || email}
        {name && email && <span className="ml-1.5 text-xs text-gray-500">{email}</span>}
      </span>
      <span className={`shrink-0 rounded-full px-2 py-0.5 text-[11px] ${RESPONSE_STYLE[response] ?? RESPONSE_STYLE.none}`}>
        {response === 'none' ? 'no reply' : response}
      </span>
    </li>
  );
}
