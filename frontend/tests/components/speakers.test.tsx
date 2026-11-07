// Motiv: speaker identification UI (People tab section, voiceprint settings, helpers).
import { beforeEach, describe, expect, mock, test } from "bun:test";
import { act, create } from "react-test-renderer";

(globalThis as any).window = (globalThis as any).window ?? {
  addEventListener() {}, removeEventListener() {}, dispatchEvent() { return true; },
};
(globalThis as any).CustomEvent = (globalThis as any).CustomEvent ?? class { type: string; detail: any; constructor(t: string, o: any) { this.type = t; this.detail = o?.detail; } };

const invokes: Array<{ cmd: string; args: any }> = [];
const toasts: string[] = [];
let speakers: any;
let profiles: any[];

mock.module("sonner", () => ({
  toast: {
    success: (m: string) => toasts.push(`ok:${m}`),
    error: (m: string, o?: any) => toasts.push(`err:${m}:${o?.description ?? ""}`),
  },
}));
mock.module("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
mock.module("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args: any) => {
    invokes.push({ cmd, args });
    switch (cmd) {
      case "speakers_get": return speakers;
      case "speakers_analyze":
        speakers = {
          analyzed: true,
          speakers: [
            { key: "S1", name: null, email: null, profile_id: null, seconds: 300 },
            { key: "S2", name: "Priya N", email: "priya@w.com", profile_id: "voice-1", seconds: 100 },
          ],
          assignments: [["t1", "S1"], ["t2", "S2"]],
        };
        return speakers;
      case "speakers_rename":
        speakers = { ...speakers, speakers: speakers.speakers.map((s: any) => s.key === args.key ? { ...s, name: args.name, email: args.email, profile_id: args.remember ? "voice-9" : null } : s) };
        return speakers;
      case "speakers_merge":
        speakers = { ...speakers, speakers: speakers.speakers.filter((s: any) => s.key !== args.from) };
        return speakers;
      case "voice_profiles_list": return profiles;
      case "voice_profiles_delete": profiles = args.id ? profiles.filter((p) => p.id !== args.id) : []; return null;
    }
    return null;
  },
}));

const { SpeakersSection, talkShare } = await import("../../src/components/MeetingDetails/SpeakersSection");
const { VoiceprintSettings } = await import("../../src/components/VoiceprintSettings");
const { speakerLabel, speakerColor, SPEAKER_COLORS } = await import("../../src/hooks/useMeetingSpeakers");

const textOf = (n: any): string => (n.children ?? []).map((c: any) => (typeof c === "string" ? c : textOf(c))).join("");
const text = (r: any) => JSON.stringify(r.toJSON());
const byLabel = (r: any, label: string) => r.root.find((n: any) => typeof n.type === "string" && n.props["aria-label"] === label);
const buttonWith = (r: any, label: string) => r.root.findAll((n: any) => n.type === "button" && textOf(n).includes(label))[0];
async function render(el: any) { let r: any; await act(async () => { r = create(el); }); return r; }

beforeEach(() => {
  invokes.length = 0; toasts.length = 0;
  speakers = { analyzed: false, speakers: [], assignments: [] };
  profiles = [
    { id: "voice-1", name: "Priya N", email: "priya@w.com", samples: 3, updated_at: "x" },
    { id: "voice-2", name: "Sam", email: null, samples: 1, updated_at: "y" },
  ];
});

describe("helpers", () => {
  test("labels, colours and talk share", () => {
    expect(speakerLabel({ key: "S3", name: null })).toBe("Speaker 3");
    expect(speakerLabel({ key: "S3", name: "  Dan " })).toBe("Dan");
    expect(speakerColor("S1")).toBe(SPEAKER_COLORS[0]);
    expect(speakerColor("S9")).toBe(SPEAKER_COLORS[0]);
    const all = [{ key: "S1", seconds: 300 }, { key: "S2", seconds: 100 }] as any;
    expect(talkShare(all[0], all)).toBe(75);
  });
});

describe("SpeakersSection", () => {
  test("identifies speakers, then shows them with recognised names", async () => {
    const r = await render(<SpeakersSection meetingId="m1" participants={[]} />);
    await act(async () => { await buttonWith(r, "Identify speakers").props.onClick(); });
    expect(invokes.some((c) => c.cmd === "speakers_analyze" && c.args.meetingId === "m1")).toBe(true);
    expect(text(r)).toContain("Speaker 1");
    expect(text(r)).toContain("recognised");
    expect(text(r)).toContain("75");
  });

  test("naming a speaker from participants passes their email and opt-in", async () => {
    speakers = { analyzed: true, assignments: [], speakers: [{ key: "S1", name: null, email: null, profile_id: null, seconds: 10 }, { key: "S2", name: null, email: null, profile_id: null, seconds: 5 }] };
    const r = await render(<SpeakersSection meetingId="m1" participants={[{ name: "Priya N", email: "priya@w.com" }]} />);
    await act(async () => byLabel(r, "Name for Speaker 1").props.onChange({ target: { value: "Priya N" } }));
    await act(async () => byLabel(r, "Recognise Speaker 1's voice").props.onChange({ target: { checked: true } }));
    await act(async () => { await buttonWith(r, "Save").props.onClick(); });
    expect(invokes.find((c) => c.cmd === "speakers_rename")?.args).toEqual({ meetingId: "m1", key: "S1", name: "Priya N", email: "priya@w.com", remember: true });
    expect(toasts).toContain("ok:Motiv will recognise Priya N in future meetings");
  });

  test("merging a split speaker", async () => {
    speakers = { analyzed: true, assignments: [], speakers: [{ key: "S1", name: "A", email: null, profile_id: null, seconds: 10 }, { key: "S2", name: null, email: null, profile_id: null, seconds: 5 }] };
    const r = await render(<SpeakersSection meetingId="m1" participants={[]} />);
    await act(async () => { await byLabel(r, "Merge Speaker 2 into").props.onChange({ target: { value: "S1" } }); });
    expect(invokes.find((c) => c.cmd === "speakers_merge")?.args).toEqual({ meetingId: "m1", from: "S2", into: "S1" });
    expect(text(r)).not.toContain("Speaker 2");
  });
});

describe("VoiceprintSettings", () => {
  test("lists and deletes voiceprints", async () => {
    const r = await render(<VoiceprintSettings />);
    expect(text(r)).toContain("Priya N");
    await act(async () => { await byLabel(r, "Delete Sam's voiceprint").props.onClick(); });
    expect(invokes.find((c) => c.cmd === "voice_profiles_delete")?.args).toEqual({ id: "voice-2" });
    await act(async () => { buttonWith(r, "Delete all voiceprints").props.onClick(); });
    await act(async () => { await buttonWith(r, "Delete all").props.onClick(); });
    expect(text(r)).toContain("No voiceprints saved.");
  });
});
