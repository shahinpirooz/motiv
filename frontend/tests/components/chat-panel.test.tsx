// Motiv: meeting chat panel loads history, asks questions and handles errors.
import { beforeEach, describe, expect, mock, test } from "bun:test";
import { act, create } from "react-test-renderer";

const invokes: Array<{ cmd: string; args: any }> = [];
const toasts: string[] = [];
let answerError: string | null = null;
let history: any[] = [];

mock.module("sonner", () => ({
  toast: {
    success: (m: string) => toasts.push(`ok:${m}`),
    error: (m: string, o?: any) => toasts.push(`err:${m}:${o?.description ?? ""}`),
  },
}));
mock.module("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args: any) => {
    invokes.push({ cmd, args });
    if (cmd === "api_get_meeting_chat") return history;
    if (cmd === "api_clear_meeting_chat") return null;
    if (cmd === "api_chat_with_meeting") {
      if (answerError) throw answerError;
      return [
        { id: "q", meeting_id: args.meetingId, role: "user", content: args.question, created_at: "1" },
        { id: "a", meeting_id: args.meetingId, role: "assistant", content: "Ship **Friday** [12:34]", created_at: "2" },
      ];
    }
    throw new Error(`unexpected ${cmd}`);
  },
}));
mock.module("react-markdown", () => ({ default: ({ children }: any) => <span data-md>{children}</span> }));
mock.module("remark-gfm", () => ({ default: () => null }));

const { ChatPanel } = await import("../../src/components/MeetingDetails/ChatPanel");

const model = { provider: "claude", model: "claude-sonnet", whisperModel: "" } as any;

async function render(props: Partial<any> = {}) {
  let r: any;
  await act(async () => {
    r = create(<ChatPanel meetingId="m1" modelConfig={model} hasTranscripts {...props} />);
  });
  return r;
}

const text = (r: any) => JSON.stringify(r.toJSON());
const byLabel = (r: any, label: string) => r.root.find((n: any) => n.props["aria-label"] === label);

beforeEach(() => {
  invokes.length = 0;
  toasts.length = 0;
  answerError = null;
  history = [];
});

describe("ChatPanel", () => {
  test("loads saved history for the meeting", async () => {
    history = [{ id: "1", meeting_id: "m1", role: "assistant", content: "Earlier answer", created_at: "1" }];
    const r = await render();
    expect(invokes[0]).toEqual({ cmd: "api_get_meeting_chat", args: { meetingId: "m1" } });
    expect(text(r)).toContain("Earlier answer");
  });

  test("asks with the configured provider and model and shows the answer", async () => {
    const r = await render();
    await act(async () => byLabel(r, "Question").props.onChange({ target: { value: "When do we ship?" } }));
    await act(async () => { byLabel(r, "Send").props.onClick(); });
    const call = invokes.find((c) => c.cmd === "api_chat_with_meeting");
    expect(call?.args).toEqual({ meetingId: "m1", question: "When do we ship?", model: "claude", modelName: "claude-sonnet" });
    expect(text(r)).toContain("When do we ship?");
    expect(text(r)).toContain("Ship **Friday** [12:34]");
  });

  test("suggestion chips ask directly", async () => {
    const r = await render();
    const chip = r.root.find((n: any) => n.type === "button" && n.props.children === "What were the key decisions?");
    await act(async () => { chip.props.onClick(); });
    expect(invokes.some((c) => c.cmd === "api_chat_with_meeting" && c.args.question === "What were the key decisions?")).toBe(true);
  });

  test("an error shows a toast and restores the draft", async () => {
    answerError = "API key not found for claude";
    const r = await render();
    await act(async () => byLabel(r, "Question").props.onChange({ target: { value: "Hi?" } }));
    await act(async () => { byLabel(r, "Send").props.onClick(); });
    expect(toasts).toContain("err:Could not answer:API key not found for claude");
    expect(byLabel(r, "Question").props.value).toBe("Hi?");
  });

  test("is disabled without a transcript or a model", async () => {
    let r = await render({ hasTranscripts: false });
    expect(byLabel(r, "Question").props.disabled).toBe(true);
    expect(text(r)).toContain("no transcript yet");
    r = await render({ modelConfig: { provider: null, model: null } });
    expect(byLabel(r, "Send").props.disabled).toBe(true);
    expect(text(r)).toContain("Choose a summary model first");
  });

  test("clear removes the conversation", async () => {
    history = [{ id: "1", meeting_id: "m1", role: "user", content: "zebra question", created_at: "1" }];
    const r = await render();
    await act(async () => { await byLabel(r, "Clear chat").props.onClick(); });
    expect(invokes.some((c) => c.cmd === "api_clear_meeting_chat")).toBe(true);
    expect(text(r)).not.toContain("zebra question");
  });
});
