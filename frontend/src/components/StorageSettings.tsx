'use client';

// Motiv: where recordings are saved and automatic expiration of old audio
// (Rust: storage module). Transcripts, summaries, chat and action items are
// always kept; only the audio files are deleted.

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { FolderOpen, HardDrive, RotateCcw, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';

export interface Retention {
  max_age_days: number | null;
  max_total_gb: number | null;
}

export interface StorageReport {
  root: string;
  default_root: string;
  retention: Retention;
  recordings: number;
  total_bytes: number;
  oldest: string | null;
  would_delete: number;
  would_free_bytes: number;
}

const AGE_OPTIONS = [null, 7, 14, 30, 60, 90, 180, 365];
const SIZE_OPTIONS = [null, 2, 5, 10, 25, 50, 100];

export function formatBytes(n: number): string {
  if (n < 1024 * 1024) return `${Math.max(0, Math.round(n / 1024))} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(0)} MB`;
  return `${(n / 1024 ** 3).toFixed(1)} GB`;
}

export function StorageSettings() {
  const [report, setReport] = useState<StorageReport | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setReport(await invoke<StorageReport>('storage_report'));
    } catch (e) {
      toast.error('Could not read storage', { description: String(e) });
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const setFolder = async (folder: string) => {
    try {
      const prefs = await invoke<any>('get_recording_preferences');
      await invoke('set_recording_preferences', { preferences: { ...prefs, save_folder: folder } });
      toast.success('New recordings will be saved there', { description: folder || 'Default folder' });
      await refresh();
    } catch (e) {
      toast.error('Could not change the folder', { description: String(e) });
    }
  };

  const chooseFolder = async () => {
    try {
      const picked = await invoke<string | null>('storage_pick_folder');
      if (picked) await setFolder(picked);
    } catch (e) {
      toast.error('Could not open the folder picker', { description: String(e) });
    }
  };

  const saveRetention = async (retention: Retention) => {
    try {
      await invoke('storage_save_retention', { retention });
      await refresh();
    } catch (e) {
      toast.error('Could not save', { description: String(e) });
    }
  };

  const cleanNow = async () => {
    setBusy(true);
    try {
      const r = await invoke<{ deleted: number; freed_bytes: number; failed: number }>('storage_cleanup_now');
      toast.success(r.deleted ? `Deleted ${r.deleted} recording(s), freed ${formatBytes(r.freed_bytes)}` : 'Nothing to delete', {
        description: r.failed ? `${r.failed} file(s) could not be deleted` : undefined,
      });
      await refresh();
    } catch (e) {
      toast.error('Cleanup failed', { description: String(e) });
    } finally {
      setBusy(false);
    }
  };

  if (!report) return <div className="h-24 animate-pulse rounded bg-gray-100" />;
  const { retention } = report;
  const isDefault = report.root === report.default_root;
  const rulesOn = retention.max_age_days != null || retention.max_total_gb != null;

  return (
    <div className="space-y-6" data-testid="storage-settings">
      <section className="space-y-3 rounded-lg border border-gray-200 bg-white p-6">
        <h3 className="text-lg font-semibold">Recordings folder</h3>
        <p className="text-sm text-gray-600">
          New recordings are saved here. A folder inside iCloud Drive, OneDrive or Google Drive also works, and it
          syncs. Existing recordings stay where they are.
        </p>
        <div className="flex items-center gap-2 rounded-md bg-gray-50 px-3 py-2 font-mono text-xs text-gray-700 break-all" aria-label="Recordings folder path">
          {report.root}
        </div>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" size="sm" onClick={chooseFolder}>
            <FolderOpen className="h-4 w-4" /> Choose folder…
          </Button>
          {!isDefault && (
            <Button variant="ghost" size="sm" onClick={() => setFolder('')}>
              <RotateCcw className="h-4 w-4" /> Use default
            </Button>
          )}
          <Button variant="ghost" size="sm" onClick={() => invoke('open_recordings_folder').catch(() => {})}>
            Open folder
          </Button>
        </div>
      </section>

      <section className="space-y-4 rounded-lg border border-gray-200 bg-white p-6">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h3 className="text-lg font-semibold">Automatic cleanup</h3>
            <p className="mt-1 text-sm text-gray-600">
              Delete old recording audio so it doesn&apos;t fill your disk. Transcripts, summaries, chat and action
              items are kept.
            </p>
          </div>
          <div className="flex items-center gap-2 whitespace-nowrap text-sm text-gray-700">
            <HardDrive className="h-4 w-4 text-gray-400" />
            {report.recordings} recordings · {formatBytes(report.total_bytes)}
          </div>
        </div>

        <label className="flex items-center justify-between gap-6 text-sm">
          <span className="font-medium text-gray-900">Delete recordings older than</span>
          <select
            className="rounded-md border border-gray-300 bg-white px-3 py-1.5 text-sm"
            value={retention.max_age_days ?? ''}
            onChange={(e) => saveRetention({ ...retention, max_age_days: e.target.value ? Number(e.target.value) : null })}
            aria-label="Delete recordings older than"
          >
            {AGE_OPTIONS.map((d) => (
              <option key={d ?? 'never'} value={d ?? ''}>{d == null ? 'Never' : d === 365 ? '1 year' : `${d} days`}</option>
            ))}
          </select>
        </label>

        <label className="flex items-center justify-between gap-6 text-sm">
          <span className="font-medium text-gray-900">Keep recordings under</span>
          <select
            className="rounded-md border border-gray-300 bg-white px-3 py-1.5 text-sm"
            value={retention.max_total_gb ?? ''}
            onChange={(e) => saveRetention({ ...retention, max_total_gb: e.target.value ? Number(e.target.value) : null })}
            aria-label="Keep recordings under"
          >
            {SIZE_OPTIONS.map((g) => (
              <option key={g ?? 'none'} value={g ?? ''}>{g == null ? 'No limit' : `${g} GB (oldest deleted first)`}</option>
            ))}
          </select>
        </label>

        {rulesOn && (
          <div className="flex items-center justify-between gap-4 rounded-md bg-amber-50 px-3 py-2 text-sm text-amber-900">
            <span>
              {report.would_delete
                ? `${report.would_delete} recording(s), ${formatBytes(report.would_free_bytes)}, will be deleted at the next cleanup.`
                : 'Nothing is due for deletion. Cleanup runs every few hours.'}
            </span>
            {report.would_delete > 0 && (
              <Button size="sm" variant="outline" onClick={cleanNow} disabled={busy}>
                <Trash2 className="h-4 w-4" /> Clean up now
              </Button>
            )}
          </div>
        )}
        {report.oldest && <p className="text-xs text-gray-500">Oldest recording: {report.oldest}</p>}
      </section>
    </div>
  );
}
