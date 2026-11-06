// Motiv: Storage and Meeting awareness settings panes.
import { beforeEach, describe, expect, mock, test } from "bun:test";
import { act, create } from "react-test-renderer";

const invokes: Array<{ cmd: string; args: any }> = [];
const toasts: string[] = [];
let report: any;
let watch: any;

mock.module("sonner", () => ({
  toast: {
    success: (m: string) => toasts.push(`ok:${m}`),
    error: (m: string, o?: any) => toasts.push(`err:${m}:${o?.description ?? ""}`),
  },
}));
mock.module("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args: any) => {
    invokes.push({ cmd, args });
    switch (cmd) {
      case "storage_report": return report;
      case "storage_save_retention": report = { ...report, retention: args.retention }; return null;
      case "storage_pick_folder": return "/Users/me/OneDrive/Motiv";
      case "get_recording_preferences": return { save_folder: "", auto_save: true, file_format: "mp4" };
      case "set_recording_preferences": report = { ...report, root: args.preferences.save_folder || report.default_root }; return null;
      case "storage_cleanup_now": return { deleted: 3, freed_bytes: 3 * 1024 ** 3, failed: 0 };
      case "meeting_watch_get_settings": return watch;
      case "meeting_watch_save_settings": watch = args.settings; return null;
      default: return null;
    }
  },
}));
mock.module("@/components/ui/switch", () => ({
  Switch: ({ checked, onCheckedChange, ...rest }: any) => (
    <button role="switch" aria-checked={checked} onClick={() => onCheckedChange(!checked)} {...rest} />
  ),
}));

const { StorageSettings, formatBytes } = await import("../../src/components/StorageSettings");
const { MeetingAwarenessSettings } = await import("../../src/components/MeetingAwarenessSettings");

async function render(el: any) {
  let r: any;
  await act(async () => { r = create(el); });
  return r;
}
const byLabel = (r: any, label: string) => r.root.find((n: any) => n.props["aria-label"] === label);
const text = (r: any) => JSON.stringify(r.toJSON());
const textOf = (n: any): string => (n.children ?? []).map((c: any) => (typeof c === "string" ? c : textOf(c))).join("");
const buttonWith = (r: any, label: string) => r.root.findAll((n: any) => n.type === "button" && textOf(n).includes(label))[0];

beforeEach(() => {
  invokes.length = 0;
  toasts.length = 0;
  report = {
    root: "/Users/me/Movies/motiv-recordings",
    default_root: "/Users/me/Movies/motiv-recordings",
    retention: { max_age_days: null, max_total_gb: null },
    recordings: 12,
    total_bytes: 6.5 * 1024 ** 3,
    oldest: "2026-03-01",
    would_delete: 0,
    would_free_bytes: 0,
  };
  watch = { enabled: true, on_start: "ask", on_end: "ask", apps: { zoom: true, teams: true, meet: true } };
});

describe("StorageSettings", () => {
  test("shows folder and usage", async () => {
    const r = await render(<StorageSettings />);
    expect(text(r)).toContain("/Users/me/Movies/motiv-recordings");
    expect(text(r)).toContain("6.5 GB");
  });

  test("choosing a folder saves it to recording preferences", async () => {
    const r = await render(<StorageSettings />);
    const btn = buttonWith(r, "Choose folder");
    await act(async () => { await btn.props.onClick(); });
    const set = invokes.find((c) => c.cmd === "set_recording_preferences");
    expect(set?.args.preferences.save_folder).toBe("/Users/me/OneDrive/Motiv");
    expect(text(r)).toContain("/Users/me/OneDrive/Motiv");
  });

  test("retention choices are saved and cleanup can run now", async () => {
    const r = await render(<StorageSettings />);
    report.would_delete = 3;
    report.would_free_bytes = 3 * 1024 ** 3;
    await act(async () => { await byLabel(r, "Delete recordings older than").props.onChange({ target: { value: "30" } }); });
    expect(invokes.find((c) => c.cmd === "storage_save_retention")?.args.retention).toEqual({ max_age_days: 30, max_total_gb: null });
    expect(text(r)).toContain("will be deleted at the next cleanup");
    const clean = buttonWith(r, "Clean up now");
    await act(async () => { await clean.props.onClick(); });
    expect(toasts.some((t) => t.startsWith("ok:Deleted 3 recording(s), freed 3.0 GB"))).toBe(true);
  });

  test("formatBytes", () => {
    expect(formatBytes(500 * 1024)).toBe("500 KB");
    expect(formatBytes(250 * 1024 ** 2)).toBe("250 MB");
    expect(formatBytes(1.25 * 1024 ** 3)).toBe("1.3 GB");
  });
});

describe("MeetingAwarenessSettings", () => {
  test("changes are saved", async () => {
    const r = await render(<MeetingAwarenessSettings />);
    await act(async () => { await byLabel(r, "When a call starts").props.onChange({ target: { value: "auto" } }); });
    expect(watch.on_start).toBe("auto");
    const teams = r.root.findAll((n: any) => n.type === "input" && n.props.type === "checkbox")[1];
    await act(async () => { await teams.props.onChange({ target: { checked: false } }); });
    expect(watch.apps).toEqual({ zoom: true, teams: false, meet: true });
  });

  test("turning it off hides the options", async () => {
    const r = await render(<MeetingAwarenessSettings />);
    const sw = r.root.find((n: any) => n.type === "button" && n.props["aria-label"] === "Meeting awareness");
    await act(async () => { await sw.props.onClick(); });
    expect(watch.enabled).toBe(false);
    expect(text(r)).not.toContain("When a call starts");
  });
});
