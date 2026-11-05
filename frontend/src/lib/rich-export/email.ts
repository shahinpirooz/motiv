/**
 * "Send" support (fork addition): opens the system's email client with the
 * meeting summary.
 *
 * Two modes, because no cross-platform link can carry a formatted body
 * (mailto: bodies are plain text and length-limited):
 *
 *  - 'draft'  Writes an unsent .eml message (HTML + plain text) and opens it
 *             with the default mail app. Classic Outlook for Windows and
 *             Thunderbird open it as an editable draft, fully formatted.
 *  - 'paste'  Opens a new message (mailto:) with the subject filled in; the
 *             formatted summary is already on the clipboard, so one paste
 *             (Ctrl+V / Cmd+V) fills the body. Works with every mail app,
 *             including Apple Mail and new Outlook.
 *
 * Default: 'draft' on Windows, 'paste' on macOS and Linux (Apple Mail and
 * Outlook for Mac open .eml files as received messages, not drafts).
 */
import { invoke } from '@tauri-apps/api/core';

export type EmailMode = 'draft' | 'paste';

export interface EmailContent {
  subject: string;
  html: string;
  plain: string;
  to?: string;
  date?: Date;
}

const MODE_KEY = 'meetily.richExport.emailMode';

export function defaultEmailMode(platform: string = detectPlatform()): EmailMode {
  return platform === 'windows' ? 'draft' : 'paste';
}

export function detectPlatform(): string {
  if (typeof navigator === 'undefined') return 'unknown';
  const ua = `${navigator.userAgent} ${(navigator as any).platform ?? ''}`.toLowerCase();
  if (ua.includes('win')) return 'windows';
  if (ua.includes('mac')) return 'macos';
  if (ua.includes('linux')) return 'linux';
  return 'unknown';
}

export function loadEmailMode(): EmailMode {
  try {
    const v = globalThis.localStorage?.getItem(MODE_KEY);
    if (v === 'draft' || v === 'paste') return v;
  } catch {
    /* storage unavailable */
  }
  return defaultEmailMode();
}

export function saveEmailMode(mode: EmailMode): void {
  try {
    globalThis.localStorage?.setItem(MODE_KEY, mode);
  } catch {
    /* storage unavailable */
  }
}

// ------------------------------------------------------------------ mailto

/** RFC 6068 mailto: URL. Spaces are %20 (not +), as mail clients expect. */
export function buildMailtoUrl(opts: { to?: string; subject?: string; body?: string }): string {
  const params: string[] = [];
  if (opts.subject) params.push(`subject=${encodeURIComponent(opts.subject)}`);
  if (opts.body) params.push(`body=${encodeURIComponent(opts.body.replace(/\r?\n/g, '\r\n'))}`);
  const to = opts.to ? encodeURIComponent(opts.to).replace(/%40/g, '@') : '';
  return `mailto:${to}${params.length ? '?' + params.join('&') : ''}`;
}

// ------------------------------------------------------------------ .eml

function base64Utf8(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let bin = '';
  for (let i = 0; i < bytes.length; i += 0x8000) {
    bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  const b64 = typeof btoa === 'function' ? btoa(bin) : Buffer.from(bin, 'binary').toString('base64');
  return b64.replace(/.{1,76}/g, '$&\r\n').trimEnd();
}

/** RFC 2047 encoded-word for non-ASCII header values. */
export function encodeHeader(value: string): string {
  const clean = value.replace(/[\r\n]+/g, ' ');
  if (/^[\x20-\x7e]*$/.test(clean)) return clean;
  const bytes = new TextEncoder().encode(clean);
  let bin = '';
  bytes.forEach(b => (bin += String.fromCharCode(b)));
  const b64 = typeof btoa === 'function' ? btoa(bin) : Buffer.from(bin, 'binary').toString('base64');
  return `=?UTF-8?B?${b64}?=`;
}

function rfc2822Date(d: Date): string {
  const days = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  const p = (n: number) => String(n).padStart(2, '0');
  const off = -d.getTimezoneOffset();
  const sign = off >= 0 ? '+' : '-';
  const tz = `${sign}${p(Math.floor(Math.abs(off) / 60))}${p(Math.abs(off) % 60)}`;
  return `${days[d.getDay()]}, ${p(d.getDate())} ${months[d.getMonth()]} ${d.getFullYear()} ` +
    `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())} ${tz}`;
}

/**
 * An unsent MIME message (multipart/alternative: plain + HTML).
 * "X-Unsent: 1" makes Outlook open it as an editable draft with Send enabled.
 */
export function buildEml(content: EmailContent, boundary = `----=_motiv_${Date.now().toString(36)}`): string {
  const html = `<!DOCTYPE html><html><head><meta charset="utf-8"></head><body>${content.html}</body></html>`;
  const headers = [
    'MIME-Version: 1.0',
    `Date: ${rfc2822Date(content.date ?? new Date())}`,
    `Subject: ${encodeHeader(content.subject)}`,
    ...(content.to ? [`To: ${encodeHeader(content.to)}`] : []),
    'X-Unsent: 1',
    `Content-Type: multipart/alternative; boundary="${boundary}"`,
  ];
  return [
    ...headers,
    '',
    'This is a multi-part message in MIME format.',
    '',
    `--${boundary}`,
    'Content-Type: text/plain; charset="utf-8"',
    'Content-Transfer-Encoding: base64',
    '',
    base64Utf8(content.plain),
    '',
    `--${boundary}`,
    'Content-Type: text/html; charset="utf-8"',
    'Content-Transfer-Encoding: base64',
    '',
    base64Utf8(html),
    '',
    `--${boundary}--`,
    '',
  ].join('\r\n');
}

/** A safe file name for the draft, e.g. "Weekly sync - summary.eml". */
export function draftFileName(subject: string): string {
  const base = subject.replace(/[\\/:*?"<>|\x00-\x1f]+/g, ' ').replace(/\s+/g, ' ').trim().slice(0, 80);
  return `${base || 'Meeting summary'}.eml`;
}

// ------------------------------------------------------------------ open

/** Opens the email client. The caller has already put the rich text on the clipboard for 'paste'. */
export async function openEmail(content: EmailContent, mode: EmailMode): Promise<void> {
  if (mode === 'draft') {
    await invoke('rich_export_open_eml', {
      eml: buildEml(content),
      fileName: draftFileName(content.subject),
    });
  } else {
    await invoke('rich_export_open_mailto', {
      url: buildMailtoUrl({ to: content.to, subject: content.subject }),
    });
  }
}
