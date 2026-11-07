'use client';

// Motiv: list and delete saved voiceprints (Rust: speakers::voice_profiles_*).

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';

export interface VoiceProfile { id: string; name: string; email: string | null; samples: number; updated_at: string }

export function VoiceprintSettings() {
  const [profiles, setProfiles] = useState<VoiceProfile[] | null>(null);
  const [confirmAll, setConfirmAll] = useState(false);

  const load = useCallback(async () => {
    try {
      setProfiles(await invoke<VoiceProfile[]>('voice_profiles_list'));
    } catch (e) {
      toast.error('Could not load voiceprints', { description: String(e) });
    }
  }, []);
  useEffect(() => { void load(); }, [load]);

  const remove = async (id: string | null) => {
    try {
      await invoke('voice_profiles_delete', { id });
      setConfirmAll(false);
      toast.success(id ? 'Voiceprint deleted' : 'All voiceprints deleted');
      await load();
    } catch (e) {
      toast.error('Could not delete', { description: String(e) });
    }
  };

  return (
    <div className="mt-6 space-y-4 rounded-lg border border-gray-200 bg-white p-6" data-testid="voiceprint-settings">
      <div>
        <h3 className="text-lg font-semibold">Voiceprints</h3>
        <p className="mt-1 text-sm text-gray-600">
          When you name a speaker and tick &ldquo;Recognise this voice&rdquo;, Motiv saves a voiceprint so it can name
          that person in later meetings. Voiceprints are biometric data: they stay in Motiv&apos;s database on this
          computer and are never uploaded. Ask people before saving theirs. Deleting one keeps the names already
          given in past meetings.
        </p>
      </div>
      {profiles === null ? null : profiles.length === 0 ? (
        <p className="text-sm text-gray-500">No voiceprints saved.</p>
      ) : (
        <>
          <ul className="divide-y divide-gray-100">
            {profiles.map((p) => (
              <li key={p.id} className="flex items-center justify-between gap-3 py-2 text-sm">
                <span className="min-w-0 truncate">
                  <span className="font-medium text-gray-900">{p.name}</span>
                  {p.email && <span className="ml-2 text-xs text-gray-500">{p.email}</span>}
                  <span className="ml-2 text-xs text-gray-400">from {p.samples} meeting{p.samples === 1 ? '' : 's'}</span>
                </span>
                <Button variant="ghost" size="sm" onClick={() => remove(p.id)} aria-label={`Delete ${p.name}'s voiceprint`}>
                  <Trash2 className="h-4 w-4" />
                </Button>
              </li>
            ))}
          </ul>
          <div className="flex justify-end gap-2">
            {confirmAll ? (
              <>
                <span className="self-center text-sm text-gray-600">Delete all {profiles.length}?</span>
                <Button variant="outline" size="sm" onClick={() => setConfirmAll(false)}>Cancel</Button>
                <Button size="sm" className="bg-red-600 hover:bg-red-700" onClick={() => remove(null)}>Delete all</Button>
              </>
            ) : (
              <Button variant="ghost" size="sm" onClick={() => setConfirmAll(true)}>Delete all voiceprints</Button>
            )}
          </div>
        </>
      )}
    </div>
  );
}
