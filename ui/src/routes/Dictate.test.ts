import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Dictate from "./Dictate.svelte";

const api = vi.hoisted(() => ({
  getRecordingStatus: vi.fn(),
  isModelReady: vi.fn(),
  loadWhisperModel: vi.fn(),
  startRecording: vi.fn(),
  stopRecording: vi.fn(),
  saveCorrections: vi.fn(),
}));
const tauri = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: any }) => void>(),
}));
vi.mock("../lib/api", async (original) => ({
  ...(await original<typeof import("../lib/api")>()),
  ...api,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, handler: (event: { payload: any }) => void) => {
    tauri.handlers.set(name, handler);
    return Promise.resolve(vi.fn());
  }),
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

async function transcript(session = "session-a", text = "original words") {
  tauri.handlers.get("transcription-complete")!({ payload: {
    session_id: session,
    raw: { full_text: text },
    corrected: { corrected: text, changes: [] },
  } });
  await screen.findByRole("button", { name: text.split(" ")[0] });
}

async function edit(word: string, replacement: string) {
  await fireEvent.click(screen.getByRole("button", { name: word }));
  const input = screen.getByRole("textbox");
  await fireEvent.input(input, { target: { value: replacement } });
  await fireEvent.keyDown(input, { key: "Enter" });
}

describe("Dictate lifecycle and review ownership", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauri.handlers.clear();
    api.getRecordingStatus.mockResolvedValue("idle");
    api.isModelReady.mockResolvedValue(true);
    api.startRecording.mockResolvedValue(undefined);
    api.stopRecording.mockResolvedValue(undefined);
    api.saveCorrections.mockResolvedValue(undefined);
  });

  it("restores the stop control when mounted during a recording", async () => {
    api.getRecordingStatus.mockResolvedValue("recording");
    render(Dictate);
    await fireEvent.click(await screen.findByRole("button", { name: /Recording.*stop/ }));
    expect(api.stopRecording).toHaveBeenCalledOnce();
    expect(api.startRecording).not.toHaveBeenCalled();
  });

  it("disables recording while a remounted backend is processing", async () => {
    api.getRecordingStatus.mockResolvedValue("processing");
    render(Dictate);
    const button = await screen.findByRole("button", { name: /Processing/ });
    expect((button as HTMLButtonElement).disabled).toBe(true);
  });

  it("does not let a stale initial status overwrite a newer event", async () => {
    const status = deferred<string>();
    api.getRecordingStatus.mockReturnValue(status.promise);
    render(Dictate);
    await waitFor(() => expect(api.getRecordingStatus).toHaveBeenCalled());
    tauri.handlers.get("recording-state")!({ payload: { state: "recording" } });
    status.resolve("idle");
    expect(await screen.findByRole("button", { name: /Recording.*stop/ })).toBeTruthy();
  });

  it("recovers the backend state after a rejected start", async () => {
    api.startRecording.mockRejectedValue(new Error("already recording"));
    api.getRecordingStatus.mockResolvedValueOnce("idle").mockResolvedValue("recording");
    render(Dictate);
    await fireEvent.click(await screen.findByRole("button", { name: /Ready.*start/ }));
    await fireEvent.click(await screen.findByRole("button", { name: /Recording.*stop/ }));
    expect(api.stopRecording).toHaveBeenCalledOnce();
  });

  it("keeps edits made during a save dirty and saves their latest text", async () => {
    const pending = deferred<void>();
    api.saveCorrections.mockReturnValueOnce(pending.promise);
    render(Dictate);
    await transcript();
    await edit("original", "first");
    await fireEvent.click(screen.getByRole("button", { name: "Save to Profile" }));
    await edit("first", "newer");
    pending.resolve();
    const save = await screen.findByRole("button", { name: "Save to Profile" });
    expect((save as HTMLButtonElement).disabled).toBe(false);
    expect(screen.queryByText("Saved to profile")).toBeNull();
    await fireEvent.click(save);
    expect(api.saveCorrections).toHaveBeenLastCalledWith("session-a", [
      { segment_index: 0, corrected_text: "newer words" },
    ]);
  });

  it("keeps a command failure visible when the backend is idle", async () => {
    api.startRecording.mockRejectedValue(new Error("No microphone input device"));
    render(Dictate);
    await fireEvent.click(await screen.findByRole("button", { name: /Ready.*start/ }));
    expect(await screen.findByText(/No microphone input device/)).toBeTruthy();
    expect(screen.getByRole("button", { name: /Error/ })).toBeTruthy();
  });

  it.each(["success", "failure"])("ignores an older session save %s", async (outcome) => {
    const pending = deferred<void>();
    api.saveCorrections.mockReturnValueOnce(pending.promise);
    render(Dictate);
    await transcript();
    await edit("original", "first");
    await fireEvent.click(screen.getByRole("button", { name: "Save to Profile" }));
    await transcript("session-b", "second transcript");
    await edit("second", "newer");
    if (outcome === "success") pending.resolve();
    else pending.reject(new Error("old save failed"));
    await waitFor(() => {
      expect((screen.getByRole("button", { name: "Save to Profile" }) as HTMLButtonElement).disabled).toBe(false);
      expect(screen.queryByText(/Saved to profile|Save failed:/)).toBeNull();
    });
  });

  it("discards an unfinished word edit when the next transcript arrives", async () => {
    render(Dictate);
    await transcript();
    await fireEvent.click(screen.getByRole("button", { name: "words" }));
    await fireEvent.input(screen.getByRole("textbox"), { target: { value: "old draft" } });
    await transcript("session-b", "next transcript");
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.getByRole("button", { name: "transcript" })).toBeTruthy();
    expect(screen.queryByText("old draft")).toBeNull();
  });
});
