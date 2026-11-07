/**
 * Update Service (Motiv)
 *
 * Checks the latest GitHub release of the Motiv repo (Rust:
 * `motiv_latest_release`). Builds aren't signed for automatic installation,
 * so an available update links to its release page.
 */

import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';

export interface UpdateInfo {
  available: boolean;
  currentVersion: string;
  version?: string;
  date?: string;
  body?: string;
  /** Release page with the downloads. */
  downloadUrl?: string;
  /** No release has been published yet. */
  noReleases?: boolean;
}

interface LatestRelease {
  version: string;
  url: string;
  notes: string | null;
  published_at: string | null;
}

/** True when `latest` (e.g. "0.8.0" or "v0.8.0") is newer than `current`. */
export function isNewer(latest: string, current: string): boolean {
  const parse = (v: string) => v.trim().replace(/^v/i, '').split(/[-+]/)[0].split('.').map((n) => parseInt(n, 10) || 0);
  const [a, b] = [parse(latest), parse(current)];
  for (let i = 0; i < 3; i++) {
    if ((a[i] ?? 0) !== (b[i] ?? 0)) return (a[i] ?? 0) > (b[i] ?? 0);
  }
  return false;
}

export class UpdateService {
  private updateCheckInProgress = false;
  private lastCheckTime: number | null = null;
  private readonly CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000; // 24 hours

  async checkForUpdates(force = false): Promise<UpdateInfo> {
    const currentVersion = await getVersion();
    if (this.updateCheckInProgress) {
      return { available: false, currentVersion };
    }
    if (!force && this.wasCheckedRecently()) {
      return { available: false, currentVersion };
    }
    this.updateCheckInProgress = true;
    this.lastCheckTime = Date.now();
    try {
      const latest = await invoke<LatestRelease | null>('motiv_latest_release');
      if (!latest) return { available: false, currentVersion, noReleases: true };
      return {
        available: isNewer(latest.version, currentVersion),
        currentVersion,
        version: latest.version,
        date: latest.published_at ?? undefined,
        body: latest.notes ?? undefined,
        downloadUrl: latest.url,
      };
    } finally {
      this.updateCheckInProgress = false;
    }
  }

  async getCurrentVersion(): Promise<string> {
    return getVersion();
  }

  wasCheckedRecently(): boolean {
    if (!this.lastCheckTime) return false;
    return Date.now() - this.lastCheckTime < this.CHECK_INTERVAL_MS;
  }
}

export const updateService = new UpdateService();
