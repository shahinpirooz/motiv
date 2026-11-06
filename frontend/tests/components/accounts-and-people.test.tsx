// Motiv: Accounts settings and the meeting People panel.
import { beforeEach, describe, expect, mock, test } from "bun:test";
import { act, create } from "react-test-renderer";

const invokes: Array<{ cmd: string; args: any }> = [];
const toasts: string[] = [];
let status: any[];
let calendar: any;
let attendanceError: string | null = null;

mock.module("sonner", () => ({
  toast: {
    success: (m: string) => toasts.push(`ok:${m}`),
    info: (m: string) => toasts.push(`info:${m}`),
    error: (m: string, o?: any) => toasts.push(`err:${m}:${o?.description ?? ""}`),
  },
}));
mock.module("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args: any) => {
    invokes.push({ cmd, args });
    switch (cmd) {
      case "accounts_status": return status;
      case "accounts_save_config": status = status.map((s) => (s.provider === args.provider ? { ...s, config: args.config } : s)); return null;
      case "accounts_connect": {
        status = status.map((s) => (s.provider === args.provider ? { ...s, connected: true, config: { ...s.config, account: "me@whitedog.com" } } : s));
        return status.find((s) => s.provider === args.provider);
      }
      case "accounts_disconnect": status = status.map((s) => (s.provider === args.provider ? { ...s, connected: false } : s)); return null;
      case "calendar_for_meeting": return calendar;
      case "calendar_set_meeting_event": return args.event ? { meeting_id: args.meetingId, event: args.event, attendance: null, source: "manual" } : null;
      case "calendar_refresh_attendance":
        if (attendanceError) throw attendanceError;
        return { ...calendar.link, attendance: [{ name: "Priya N", email: "priya@w.com", role: "Presenter", seconds: 1740 }] };
      default: return null;
    }
  },
}));

const { AccountsSettings } = await import("../../src/components/AccountsSettings");
const { PeoplePanel, minutes } = await import("../../src/components/MeetingDetails/PeoplePanel");

const textOf = (n: any): string => (n.children ?? []).map((c: any) => (typeof c === "string" ? c : textOf(c))).join("");
const text = (r: any) => JSON.stringify(r.toJSON());
const byLabel = (r: any, label: string) => r.root.find((n: any) => typeof n.type === "string" && n.props["aria-label"] === label);
const buttonWith = (r: any, label: string, scope?: any) =>
  (scope ?? r.root).findAll((n: any) => n.type === "button" && textOf(n).includes(label))[0];
async function render(el: any) { let r: any; await act(async () => { r = create(el); }); return r; }

const cfg = (o: any = {}) => ({ client_id: "", client_secret: "", tenant: "organizations", teams_attendance: true, account: null, ...o });
const event = (id: string, title: string, platform: string | null = "teams") => ({
  provider: "microsoft", id, title, start: "2026-11-06T17:00:00Z", end: "2026-11-06T17:30:00Z", all_day: false,
  organizer: { name: "Shahin Pirooz", email: "shahin@w.com" },
  attendees: [{ name: "Priya N", email: "priya@w.com", response: "accepted" }, { name: "Dan", email: "dan@w.com", response: "declined" }],
  join_url: "https://teams.microsoft.com/l/x", platform,
});

beforeEach(() => {
  invokes.length = 0; toasts.length = 0; attendanceError = null;
  status = [
    { provider: "microsoft", label: "Microsoft 365", config: cfg(), connected: false },
    { provider: "google", label: "Google", config: cfg({ tenant: "" }), connected: false },
  ];
  calendar = { connected: true, link: { meeting_id: "m1", event: event("e1", "Weekly Planning"), attendance: null, source: "auto" }, candidates: [event("e1", "Weekly Planning"), event("e2", "Standup", null)] };
});

describe("AccountsSettings", () => {
  test("connect saves the client ID first, then signs in", async () => {
    const r = await render(<AccountsSettings />);
    await act(async () => byLabel(r, "Microsoft 365 client ID").props.onChange({ target: { value: "abc-123" } }));
    const ms = r.root.find((n: any) => n.props["data-testid"] === "provider-microsoft");
    await act(async () => { await buttonWith(r, "Connect", ms).props.onClick(); });
    const names = invokes.map((c) => c.cmd);
    expect(names.indexOf("accounts_save_config")).toBeLessThan(names.indexOf("accounts_connect"));
    expect(invokes.find((c) => c.cmd === "accounts_save_config")?.args.config.client_id).toBe("abc-123");
    expect(text(r)).toContain("me@whitedog.com");
    expect(toasts).toContain("ok:Microsoft 365 connected");
  });

  test("connect is disabled without a client ID; Google asks for a secret", async () => {
    const r = await render(<AccountsSettings />);
    const g = r.root.find((n: any) => n.props["data-testid"] === "provider-google");
    expect(buttonWith(r, "Connect", g).props.disabled).toBe(true);
    expect(byLabel(r, "Google client secret").props.type).toBe("password");
  });
});

describe("PeoplePanel", () => {
  test("shows the matched event, organizer and responses", async () => {
    const r = await render(<PeoplePanel meetingId="m1" />);
    expect(byLabel(r, "Calendar event").props.value).toBe("microsoft:e1");
    expect(text(r)).toContain("matched by time");
    expect(text(r)).toContain("Shahin Pirooz");
    expect(text(r)).toContain("declined");
  });

  test("picking another event saves it manually", async () => {
    const r = await render(<PeoplePanel meetingId="m1" />);
    await act(async () => { await byLabel(r, "Calendar event").props.onChange({ target: { value: "microsoft:e2" } }); });
    expect(invokes.find((c) => c.cmd === "calendar_set_meeting_event")?.args.event.id).toBe("e2");
    expect(text(r)).not.toContain("Get attendance");
  });

  test("Teams attendance", async () => {
    const r = await render(<PeoplePanel meetingId="m1" />);
    await act(async () => { await byLabel(r, "Get attendance").props.onClick(); });
    expect(text(r)).toContain("Presenter · ");
    expect(text(r)).toContain("29 min");
    attendanceError = "403 denied";
    await act(async () => { await byLabel(r, "Get attendance").props.onClick(); });
    expect(toasts).toContain("err:Could not get attendance:403 denied");
  });

  test("not connected", async () => {
    calendar = { connected: false, link: null, candidates: [] };
    const r = await render(<PeoplePanel meetingId="m1" />);
    expect(text(r)).toContain("Settings > Accounts");
  });

  test("minutes", () => {
    expect(minutes(45)).toBe("45s");
    expect(minutes(1740)).toBe("29 min");
  });
});
