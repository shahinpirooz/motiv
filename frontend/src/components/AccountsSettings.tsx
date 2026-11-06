'use client';

// Motiv: connect Microsoft 365 and Google for calendars and participants
// (Rust: accounts, calendar). Uses the user's own app registrations.

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { CheckCircle2, ChevronRight, ExternalLink, Loader2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';

export type ProviderKey = 'microsoft' | 'google';

export interface ProviderConfig {
  client_id: string;
  client_secret: string;
  tenant: string;
  teams_attendance: boolean;
  account: string | null;
}

export interface ProviderStatus {
  provider: ProviderKey;
  label: string;
  config: ProviderConfig;
  connected: boolean;
  /** This copy of Motiv has a built-in app registration for the provider. */
  built_in: boolean;
}

const GUIDE_URL = 'https://github.com/shahinpirooz/motiv/blob/main/docs/CALENDAR_SETUP.md';

export function AccountsSettings() {
  const [status, setStatus] = useState<ProviderStatus[] | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<ProviderStatus[]>('accounts_status'));
    } catch (e) {
      toast.error('Could not load accounts', { description: String(e) });
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  if (!status) return <div className="h-24 animate-pulse rounded bg-gray-100" />;

  return (
    <div className="space-y-6" data-testid="accounts-settings">
      <div className="rounded-lg border border-gray-200 bg-white p-6">
        <h3 className="text-lg font-semibold">Calendars &amp; participants</h3>
        <p className="mt-1 text-sm text-gray-600">
          Connect your calendars so Motiv names recordings after the meeting, knows who was invited, and (for Teams
          meetings you organise) who actually attended. You sign in in your browser; Motiv only reads, and the
          sign-in stays in this computer&apos;s keychain.
        </p>
        <button
          type="button"
          onClick={() => invoke('open_external_url', { url: GUIDE_URL }).catch(() => {})}
          className="mt-2 inline-flex items-center gap-1 text-sm text-blue-600 hover:underline"
        >
          Setup guide <ExternalLink className="h-3.5 w-3.5" />
        </button>
      </div>
      {status.map((s) => (
        <ProviderCard key={s.provider} status={s} onChange={refresh} />
      ))}
    </div>
  );
}

function ProviderCard({ status, onChange }: { status: ProviderStatus; onChange: () => Promise<void> }) {
  const [cfg, setCfg] = useState<ProviderConfig>(status.config);
  const [busy, setBusy] = useState<'save' | 'connect' | 'disconnect' | null>(null);
  const isMs = status.provider === 'microsoft';
  const custom = cfg.client_id.trim().length > 0;
  const canConnect = status.built_in || (custom && (isMs || cfg.client_secret.trim().length > 0));
  const [advanced, setAdvanced] = useState(!status.built_in || Boolean(status.config.client_id));
  const dirty = JSON.stringify(cfg) !== JSON.stringify(status.config);

  useEffect(() => { setCfg(status.config); }, [status.config]);

  const save = async () => {
    await invoke('accounts_save_config', { provider: status.provider, config: cfg });
  };

  const run = async (kind: 'save' | 'connect' | 'disconnect') => {
    setBusy(kind);
    try {
      if (kind === 'save') {
        await save();
        toast.success('Saved');
      } else if (kind === 'connect') {
        if (dirty) await save();
        toast.info(`Finish signing in to ${status.label} in your browser`);
        const s = await invoke<ProviderStatus>('accounts_connect', { provider: status.provider });
        toast.success(`${status.label} connected`, { description: s.config.account ?? undefined });
      } else {
        await invoke('accounts_disconnect', { provider: status.provider });
        toast.success(`${status.label} disconnected`);
      }
      await onChange();
    } catch (e) {
      toast.error(kind === 'connect' ? `Could not connect ${status.label}` : 'Something went wrong', { description: String(e) });
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="space-y-4 rounded-lg border border-gray-200 bg-white p-6" data-testid={`provider-${status.provider}`}>
      <div className="flex items-start justify-between gap-4">
        <div>
          <h3 className="text-base font-semibold">{status.label}</h3>
          <p className="text-sm text-gray-600">
            {isMs ? 'Outlook calendar, Teams meetings and attendance' : 'Google Calendar and Google Meet'}
          </p>
        </div>
        {status.connected ? (
          <span className="flex items-center gap-1.5 rounded-full bg-green-50 px-3 py-1 text-xs font-medium text-green-700">
            <CheckCircle2 className="h-3.5 w-3.5" /> {status.config.account ?? 'Connected'}
          </span>
        ) : (
          <span className="rounded-full bg-gray-100 px-3 py-1 text-xs text-gray-600">Not connected</span>
        )}
      </div>

      {isMs && (
        <label className="flex items-start gap-2 text-sm text-gray-700">
          <input
            type="checkbox"
            className="mt-0.5 h-4 w-4 rounded border-gray-300"
            checked={cfg.teams_attendance}
            onChange={(e) => setCfg({ ...cfg, teams_attendance: e.target.checked })}
            aria-label="Teams attendance reports"
          />
          <span>
            Also read Teams attendance reports (who actually joined). Work accounts only, and your organisation&apos;s
            admin usually has to approve it. Reconnect after changing this.
          </span>
        </label>
      )}

      {!canConnect && (
        <p className="rounded-md bg-amber-50 px-3 py-2 text-sm text-amber-900">
          This copy of Motiv has no built-in {status.label} app registration yet. Add one under Advanced (see the
          setup guide).
        </p>
      )}

      <div className="flex flex-wrap gap-2">
        <Button onClick={() => run('connect')} disabled={!canConnect || busy !== null}>
          {busy === 'connect' && <Loader2 className="h-4 w-4 animate-spin" />}
          {busy === 'connect' ? 'Waiting for browser…' : status.connected ? 'Reconnect' : `Connect ${status.label}`}
        </Button>
        {dirty && (
          <Button variant="outline" onClick={() => run('save')} disabled={busy !== null}>Save</Button>
        )}
        {status.connected && (
          <Button variant="ghost" onClick={() => run('disconnect')} disabled={busy !== null}>Disconnect</Button>
        )}
      </div>

      <div className="border-t border-gray-100 pt-3">
        <button
          type="button"
          onClick={() => setAdvanced((v) => !v)}
          className="flex items-center gap-1 text-xs font-medium text-gray-500 hover:text-gray-900"
          aria-expanded={advanced}
        >
          <ChevronRight className={`h-3.5 w-3.5 transition-transform ${advanced ? 'rotate-90' : ''}`} />
          Advanced: use your own app registration
        </button>
        {advanced && (
          <div className="mt-3 space-y-3">
            <p className="text-xs text-gray-500">
              Leave empty to use Motiv&apos;s built-in registration. Some organisations require their own; see the
              setup guide.
            </p>
            <div className="grid gap-3 sm:grid-cols-2">
              <Field label={isMs ? 'Application (client) ID' : 'Client ID'}>
                <Input
                  value={cfg.client_id}
                  onChange={(e) => setCfg({ ...cfg, client_id: e.target.value })}
                  placeholder={status.built_in ? 'Built-in' : isMs ? '00000000-0000-0000-0000-000000000000' : '1234-abc.apps.googleusercontent.com'}
                  aria-label={`${status.label} client ID`}
                />
              </Field>
              {isMs ? (
                <Field label="Tenant">
                  <Input
                    value={cfg.tenant}
                    onChange={(e) => setCfg({ ...cfg, tenant: e.target.value })}
                    placeholder="organizations"
                    aria-label="Microsoft tenant"
                  />
                </Field>
              ) : (
                <Field label="Client secret">
                  <Input
                    type="password"
                    value={cfg.client_secret}
                    onChange={(e) => setCfg({ ...cfg, client_secret: e.target.value })}
                    aria-label="Google client secret"
                  />
                </Field>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block space-y-1 text-sm">
      <span className="font-medium text-gray-900">{label}</span>
      {children}
    </label>
  );
}
