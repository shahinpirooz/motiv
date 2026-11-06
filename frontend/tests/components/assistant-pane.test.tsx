// Motiv: app-wide assistant pane (meeting / all-meetings chat).
import { beforeEach, describe, expect, mock, test } from "bun:test";
import { act, create } from "react-test-renderer";

// Minimal browser globals (the test runner has no DOM).
const store: Record<string, string> = {};
(globalThis as any).localStorage = {
  getItem: (k: string) => (k in store ? store[k] : null),
  setItem: (k: string, v: string) => { store[k] = String(v); },
  clear: () => { for (const k of Object.keys(store)) delete store[k]; },
};
(globalThis as any).window = (globalThis as any).window ?? { location: { search: "" }, addEventListener() {}, removeEventListener() {} };
const setSearch = (s: string) => { (globalThis as any).window.location = { search: s }; };

const invokes: Array<{ cmd: string; args: any }> = [];
const toasts: string[] = [];
const pushes: string[] = [];
let pathname = "/";
let modelConfig: any = { provider: "claude", model: "claude-sonnet" };
let answerError: string | null = null;
const histories: Record<string, any[]> = {};

mock.module("sonner", () => ({
  toast: {
    success: (m: string) => toasts.push(`ok:${m}`),
    error: (m: string, o?: any) => toasts.push(`err:${m}:${o?.description ?? ""}`),
  },
}));
mock.module("next/navigation", () => ({
  usePathname: () => pathname,
  useRouter: () => ({ push: (p: string) => pushes.push(p) }),
}));
const setCurrentMeeting = mock(() => {});
mock.module("@/components/Sidebar/SidebarProvider", () => ({
  useSidebar: () => ({ currentMeeting: { id: "m-1", title: "Weekly Planning" }, setCurrentMeeting }),
}));
mock.module("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args: any) => {
    invokes.push({ cmd, args });
    switch (cmd) {
      case "api_get_model_config": return modelConfig;
      case "api_get_meeting_chat": return histories[`m:${args.meetingId}`] ?? [];
      case "api_get_assistant_chat": return histories.all ?? [];
      case "api_clear_assistant_chat": case "api_clear_meeting_chat": return null;
      case "api_chat_with_meeting":
      case "api_chat_all_meetings":
        if (answerError) throw answerError;
        return [
          { id: "q", role: "user", content: args.question },
          { id: "a", role: "assistant", content: "Ship Friday (Weekly Planning, 12:34)",
            sources: cmd === "api_chat_all_meetings" ? [{ meeting_id: "m-1", title: "Weekly Planning", date: "Mon" }] : undefined },
        ];
    }
    throw new Error(`unexpected ${cmd}`);
  },
}));
mock.module("react-markdown", () => ({ default: ({ children }: any) => <span data-md>{children}</span> }));
mock.module("remark-gfm", () => ({ default: () => null }));

const { AssistantPane, meetingIdFromLocation } = await import("../../src/components/Assistant/AssistantPane");

const textOf = (n: any): string => (n.children ?? []).map((c: any) => (typeof c === "string" ? c : textOf(c))).join("");
const text = (r: any) => JSON.stringify(r.toJSON());
const byLabel = (r: any, label: string) => r.root.find((n: any) => n.type !== undefined && typeof n.type === "string" && n.props["aria-label"] === label);
const buttonWith = (r: any, label: string) => r.root.findAll((n: any) => n.type === "button" && textOf(n).includes(label))[0];

async function render() {
  let r: any;
  await act(async () => { r = create(<AssistantPane />); });
  return r;
}
async function ask(r: any, q: string) {
  await act(async () => byLabel(r, "Question").props.onChange({ target: { value: q } }));
  await act(async () => { await byLabel(r, "Send").props.onClick(); });
}

beforeEach(() => {
  invokes.length = 0; toasts.length = 0; pushes.length = 0;
  pathname = "/"; modelConfig = { provider: "claude", model: "claude-sonnet" }; answerError = null;
  for (const k of Object.keys(histories)) delete histories[k];
  localStorage.clear();
  setSearch("");
});

describe("meetingIdFromLocation", () => {
  test("only on meeting pages", () => {
    expect(meetingIdFromLocation("/meeting-details", "?id=m-1")).toBe("m-1");
    expect(meetingIdFromLocation("/meeting-details", "?id=intro-call")).toBe(null);
    expect(meetingIdFromLocation("/", "?id=m-1")).toBe(null);
  });
});

describe("AssistantPane", () => {
  test("outside a meeting it chats across all meetings and links sources", async () => {
    const r = await render();
    expect(text(r)).toContain("Using claude-sonnet");
    expect(text(r)).not.toContain("Chat scope");
    await ask(r, "What did we decide?");
    const call = invokes.find((c) => c.cmd === "api_chat_all_meetings");
    expect(call?.args).toEqual({ question: "What did we decide?" });
    expect(text(r)).toContain("Ship Friday");
    await act(async () => { buttonWith(r, "Weekly Planning").props.onClick(); });
    expect(pushes).toEqual(["/meeting-details?id=m-1"]);
  });

  test("inside a meeting it defaults to that meeting and can switch to all", async () => {
    pathname = "/meeting-details";
    setSearch("?id=m-1");
    const r = await render();
    expect(text(r)).toContain("Weekly Planning");
    await ask(r, "Next steps?");
    expect(invokes.find((c) => c.cmd === "api_chat_with_meeting")?.args).toEqual({ meetingId: "m-1", question: "Next steps?" });
    await act(async () => { buttonWith(r, "All meetings").props.onClick(); });
    expect(invokes.some((c) => c.cmd === "api_get_assistant_chat")).toBe(true);
  });

  test("errors show a toast and keep the draft", async () => {
    answerError = "There are no meetings yet.";
    const r = await render();
    await ask(r, "Hi?");
    expect(toasts).toContain("err:Could not answer:There are no meetings yet.");
    expect(byLabel(r, "Question").props.value).toBe("Hi?");
  });

  test("without a model it explains why", async () => {
    modelConfig = { provider: null, model: null };
    const r = await render();
    expect(text(r)).toContain("Choose a summary model in Settings first.");
    expect(byLabel(r, "Send").props.disabled).toBe(true);
  });

  test("can be closed and reopened", async () => {
    const r = await render();
    await act(async () => { byLabel(r, "Close assistant").props.onClick(); });
    expect(text(r)).not.toContain("Assistant\"");
    await act(async () => { byLabel(r, "Open assistant").props.onClick(); });
    expect(r.root.findAll((n: any) => n.props["data-testid"] === "assistant-pane").length).toBeGreaterThan(0);
  });
});
