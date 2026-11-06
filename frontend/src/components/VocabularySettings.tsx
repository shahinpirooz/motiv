'use client';

// Motiv: names and terms with their correct spelling (Rust: vocabulary).

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';

export function parseTerms(text: string): string[] {
  return text
    .split(/[\n,;]/)
    .map((t) => t.trim())
    .filter(Boolean);
}

export function VocabularySettings() {
  const [text, setText] = useState('');
  const [saved, setSaved] = useState('');
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    invoke<{ terms: string[] }>('vocabulary_get')
      .then((v) => {
        const t = v.terms.join('\n');
        setText(t);
        setSaved(t);
      })
      .catch((e) => console.error('Failed to load vocabulary:', e));
  }, []);

  const save = async () => {
    setSaving(true);
    try {
      const v = await invoke<{ terms: string[] }>('vocabulary_save', { terms: parseTerms(text) });
      const t = v.terms.join('\n');
      setText(t);
      setSaved(t);
      toast.success('Names and terms saved', { description: `${v.terms.length} saved. Applies to new summaries, chat and action items.` });
    } catch (e) {
      toast.error('Could not save', { description: String(e) });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="mt-6 space-y-3 rounded-lg border border-gray-200 bg-white p-6" data-testid="vocabulary-settings">
      <div>
        <h3 className="text-lg font-semibold">Names &amp; terms</h3>
        <p className="mt-1 text-sm text-gray-600">
          People, companies and jargon spelled the way you want them, one per line. Summaries, chat and action items
          always use these spellings, even when the transcript gets them wrong. With a Whisper model they also help
          the transcript itself.
        </p>
      </div>
      <Textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        rows={6}
        placeholder={'Shahin Pirooz\nWhiteDog\nOpenXDR'}
        className="font-mono text-sm"
        aria-label="Names and terms"
      />
      <div className="flex justify-end">
        <Button onClick={save} disabled={saving || text === saved}>
          {saving ? 'Saving…' : 'Save'}
        </Button>
      </div>
    </div>
  );
}
