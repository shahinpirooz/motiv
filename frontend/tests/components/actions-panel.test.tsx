// Motiv: action items panel generates, renders, ticks off and copies insights.
import { beforeEach, describe, expect, mock, test } from "bun:test";
import { act, create } from "react-test-renderer";

const invokes: Array<{ cmd: string; args: any }> = [];
const toasts: string[] = [];
let stored: any = null;

const sample = {
  meeting_id: "m1",
  model: "claude-sonnet",
  updated_at: "2026-11-05T00:00:00Z",
  insights: {
    action_items: [
      { task: "Send the deck", owner: "Priya", due: "Friday", timestamp: "12:34", done: false },
      { task: "Book the room", owner: null, due: null, timestamp: null, done: true },
    ],
    decisions: [{ text: "Ship v2 in March", timestamp: "20:01" }],
    highlights: [],
    open_questions: [{ text: "Who owns QA?", timestamp: null }],
  },
};

mock.module("sonner", () => ({
  toast: {
    success: (m: string) => toasts.push(`ok:${m}`),
    error: (m: string, o?: any) => toasts.push(`err:${m}:${o?.description ?? ""}`),
  },
}));
mock.module("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args: any) => {
    invokes.push({ cmd, args });
    if (cmd === "api_get_meeting_insights") return stored;
    if (cmd === "api_generate_meeting_insights") return sample;
    if (cmd === "api_set_action_item_done") return null;
    throw new Error(`unexpected ${cmd}`);
  },
}));

const { ActionsPanel, insightsToMarkdown } = await import("../../src/components/MeetingDetails/ActionsPanel");
const model = { provider: "claude", model: "claude-sonnet", whisperModel: "" } as any;

async function render(props: Partial<any> = {}) {
  let r: any;
  await act(async () => {
    r = create(<ActionsPanel meetingId="m1" modelConfig={model} hasTranscripts {...props} />);
  });
  return r;
}
const text = (r: any) => JSON.stringify(r.toJSON());

beforeEach(() => {
  invokes.length = 0;
  toasts.length = 0;
  stored = null;
});

describe("ActionsPanel", () => {
  test("extracts on demand with the configured model", async () => {
    const r = await render();
    const btn = r.root.find((n: any) => n.type === "button" && ([] as unknown[]).concat(n.props.children).includes("Extract"));
    await act(async () => { await btn.props.onClick(); });
    const call = invokes.find((c) => c.cmd === "api_generate_meeting_insights");
    expect(call?.args).toEqual({ meetingId: "m1", model: "claude", modelName: "claude-sonnet" });
    expect(text(r)).toContain("Send the deck");
    expect(text(r)).toContain("Ship v2 in March");
    expect(text(r)).toContain("Who owns QA?");
  });

  test("ticking an item saves it by index", async () => {
    stored = sample;
    const r = await render();
    const box = r.root.find((n: any) => n.type === "input" && n.props["aria-label"] === "Send the deck");
    await act(async () => { await box.props.onChange({ target: { checked: true } }); });
    expect(invokes.find((c) => c.cmd === "api_set_action_item_done")?.args).toEqual({ meetingId: "m1", index: 0, done: true });
    expect(r.root.find((n: any) => n.type === "input" && n.props["aria-label"] === "Send the deck").props.checked).toBe(true);
  });

  test("disabled without a model", async () => {
    const r = await render({ modelConfig: { provider: null, model: null } });
    expect(text(r)).toContain("Choose a summary model first");
  });

  test("markdown export", () => {
    expect(insightsToMarkdown(sample.insights)).toBe(
      [
        "## Action items",
        "- [ ] Send the deck (Priya, due Friday) [12:34]",
        "- [x] Book the room",
        "",
        "## Decisions",
        "- Ship v2 in March [20:01]",
        "",
        "## Open questions",
        "- Who owns QA?",
      ].join("\n"),
    );
  });
});
