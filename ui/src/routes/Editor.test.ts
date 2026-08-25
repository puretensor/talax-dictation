import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Editor from "./Editor.svelte";
import type { SessionDetail, SessionSummary } from "../lib/api";

const api = vi.hoisted(() => ({
  getSession: vi.fn(),
  getSessions: vi.fn(),
  saveCorrections: vi.fn(),
}));

const tauri = vi.hoisted(() => ({
  listen: vi.fn(),
  profileDataChanged: undefined as undefined | (() => Promise<void>),
}));

vi.mock("../lib/api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/api")>()),
  ...api,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: tauri.listen.mockImplementation(
    (_event: string, handler: () => Promise<void>) => {
      tauri.profileDataChanged = handler;
      return Promise.resolve(vi.fn());
    }
  ),
}));

function summary(
  id: string,
  duration: number,
  createdAt = "2026-07-17T10:00:00Z"
): SessionSummary {
  return {
    id,
    created_at: createdAt,
    duration,
    segment_count: 1,
    reviewed: false,
  };
}

function detail(id: string, text: string): SessionDetail {
  return {
    id,
    created_at: "2026-07-17T10:00:00Z",
    duration: 4,
    whisper_model: "small.en-q5_1",
    segments: [
      {
        segment_index: 0,
        original_text: text,
        corrected_text: null,
        start_time: 0,
        end_time: 4,
        reviewed: false,
      },
    ],
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((complete, fail) => {
    resolve = complete;
    reject = fail;
  });
  return { promise, resolve, reject };
}

describe("Editor request ordering", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauri.profileDataChanged = undefined;
    api.getSessions.mockResolvedValue([
      summary("session-1", 4),
      summary("session-2", 5, "2026-07-17T11:00:00Z"),
    ]);
  });

  it("keeps the newest expanded session when detail requests finish out of order", async () => {
    const first = deferred<SessionDetail>();
    const second = deferred<SessionDetail>();
    api.getSession.mockImplementation((id: string) =>
      id === "session-1" ? first.promise : second.promise
    );
    render(Editor);

    const headers = await screen.findAllByRole("button", { name: /1 segment/ });
    await fireEvent.click(headers[0]);
    await fireEvent.click(
      (await screen.findAllByRole("button", { name: /1 segment/ }))[1]
    );

    second.resolve(detail("session-2", "newest session detail"));
    expect(await screen.findByText("newest session detail")).toBeTruthy();

    first.resolve(detail("session-1", "stale session detail"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    await waitFor(() => {
      expect(screen.queryByText("stale session detail")).toBeNull();
      expect(screen.getByText("newest session detail")).toBeTruthy();
    });
  });

  it("keeps the newest profile refresh when callbacks finish out of order", async () => {
    api.getSession
      .mockResolvedValueOnce(detail("session-1", "initial detail"))
      .mockResolvedValue(detail("session-1", "newest refreshed detail"));
    render(Editor);

    const firstHeader = (await screen.findAllByRole("button", { name: /1 segment/ }))[0];
    await fireEvent.click(firstHeader);
    expect(await screen.findByText("initial detail")).toBeTruthy();
    expect(tauri.profileDataChanged).toBeTypeOf("function");

    const olderSessions = deferred<SessionSummary[]>();
    const newerSessions = deferred<SessionSummary[]>();
    api.getSessions
      .mockReset()
      .mockReturnValueOnce(olderSessions.promise)
      .mockReturnValueOnce(newerSessions.promise);

    const olderRefresh = tauri.profileDataChanged!();
    const newerRefresh = tauri.profileDataChanged!();
    newerSessions.resolve([summary("session-1", 9)]);
    await newerRefresh;
    olderSessions.resolve([summary("session-1", 99)]);
    await olderRefresh;

    expect(screen.getByRole("button", { name: /9s/ })).toBeTruthy();
    expect(screen.getByText("newest refreshed detail")).toBeTruthy();
    expect(api.getSession).toHaveBeenCalledTimes(2);
  });

  it("does not resume a profile refresh after teardown", async () => {
    api.getSession.mockResolvedValue(detail("session-1", "initial detail"));
    const { unmount } = render(Editor);

    const firstHeader = (await screen.findAllByRole("button", { name: /1 segment/ }))[0];
    await fireEvent.click(firstHeader);
    expect(await screen.findByText("initial detail")).toBeTruthy();

    const pendingSessions = deferred<SessionSummary[]>();
    api.getSessions.mockReset().mockReturnValueOnce(pendingSessions.promise);
    const refresh = tauri.profileDataChanged!();
    await unmount();
    pendingSessions.resolve([summary("session-1", 8)]);
    await refresh;

    expect(api.getSession).toHaveBeenCalledTimes(1);
  });

  it("invalidates an in-flight detail as soon as profile data changes", async () => {
    const staleDetail = deferred<SessionDetail>();
    const failedRefresh = deferred<SessionSummary[]>();
    api.getSession.mockReturnValueOnce(staleDetail.promise);
    render(Editor);

    await fireEvent.click(
      (await screen.findAllByRole("button", { name: /1 segment/ }))[0]
    );
    api.getSessions.mockReset().mockReturnValueOnce(failedRefresh.promise);
    const refresh = tauri.profileDataChanged!();

    staleDetail.resolve(detail("session-1", "pre-change stale detail"));
    failedRefresh.reject(new Error("refresh unavailable"));
    await refresh;

    expect(screen.queryByText("pre-change stale detail")).toBeNull();
  });

  it("keeps loading owned by the newest overlapping profile refresh", async () => {
    api.getSession.mockResolvedValueOnce(detail("session-1", "initial detail"));
    render(Editor);
    await fireEvent.click(
      (await screen.findAllByRole("button", { name: /1 segment/ }))[0]
    );
    expect(await screen.findByText("initial detail")).toBeTruthy();

    const olderSessions = deferred<SessionSummary[]>();
    const newerSessions = deferred<SessionSummary[]>();
    const newestDetail = deferred<SessionDetail>();
    api.getSessions
      .mockReset()
      .mockReturnValueOnce(olderSessions.promise)
      .mockReturnValueOnce(newerSessions.promise);
    api.getSession.mockReset().mockReturnValueOnce(newestDetail.promise);

    const olderRefresh = tauri.profileDataChanged!();
    const newerRefresh = tauri.profileDataChanged!();
    newerSessions.resolve([summary("session-1", 9)]);
    await waitFor(() => expect(api.getSession).toHaveBeenCalledTimes(1));
    expect(screen.getByText("Loading segments...")).toBeTruthy();

    olderSessions.resolve([summary("session-1", 99)]);
    await olderRefresh;
    expect(screen.getByText("Loading segments...")).toBeTruthy();

    newestDetail.resolve(detail("session-1", "newest detail"));
    await newerRefresh;
    expect(await screen.findByText("newest detail")).toBeTruthy();
  });

  it("collapses an expansion removed by the winning session refresh", async () => {
    api.getSession.mockResolvedValue(detail("session-1", "initial detail"));
    render(Editor);

    await fireEvent.click(
      (await screen.findAllByRole("button", { name: /1 segment/ }))[0]
    );
    expect(await screen.findByText("initial detail")).toBeTruthy();

    api.getSessions.mockReset().mockResolvedValue([
      summary("session-2", 5, "2026-07-17T11:00:00Z"),
    ]);
    await tauri.profileDataChanged!();

    expect(screen.queryByText("initial detail")).toBeNull();
    expect(api.getSession).toHaveBeenCalledTimes(1);
  });

  it("clears detail loading when the backend returns no session", async () => {
    const missingDetail = deferred<SessionDetail | null>();
    api.getSession.mockReturnValueOnce(missingDetail.promise);
    render(Editor);

    await fireEvent.click(
      (await screen.findAllByRole("button", { name: /1 segment/ }))[0]
    );
    expect(screen.getByText("Loading segments...")).toBeTruthy();

    missingDetail.resolve(null);
    await waitFor(() => {
      expect(screen.queryByText("Loading segments...")).toBeNull();
      expect(screen.queryByLabelText("Corrected")).toBeNull();
    });
  });
});

describe("Editor mutation failures", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauri.profileDataChanged = undefined;
    api.getSessions.mockResolvedValue([
      summary("session-1", 4),
    ]);
    api.getSession.mockResolvedValue(detail("session-1", "the cluser"));
  });

  it("reports a save failure and preserves edits for retry", async () => {
    api.saveCorrections.mockRejectedValue(new Error("database locked"));
    render(Editor);

    await fireEvent.click(
      await screen.findByRole("button", { name: /1 segment/ })
    );
    const input = (await screen.findByLabelText("Corrected")) as HTMLTextAreaElement;
    await fireEvent.input(input, { target: { value: "the cluster" } });

    const save = screen.getByRole("button", { name: "Save Corrections" });
    await fireEvent.click(save);

    expect((await screen.findByRole("alert")).textContent).toContain(
      "Save failed: database locked"
    );
    await waitFor(() => expect(save.hasAttribute("disabled")).toBe(false));
    expect(input.value).toBe("the cluster");
    expect(api.getSessions).toHaveBeenCalledTimes(1);
    expect(api.getSession).toHaveBeenCalledTimes(1);
  });

  it("preserves edits typed while a successful save is in flight", async () => {
    const firstSave = deferred<void>();
    api.saveCorrections
      .mockReturnValueOnce(firstSave.promise)
      .mockResolvedValueOnce(undefined);
    render(Editor);

    await fireEvent.click(
      await screen.findByRole("button", { name: /1 segment/ })
    );
    const input = (await screen.findByLabelText("Corrected")) as HTMLTextAreaElement;
    await fireEvent.input(input, { target: { value: "correction A" } });
    const save = screen.getByRole("button", { name: "Save Corrections" });
    await fireEvent.click(save);
    await waitFor(() =>
      expect(api.saveCorrections).toHaveBeenCalledWith("session-1", [
        { segment_index: 0, corrected_text: "correction A" },
      ])
    );

    await fireEvent.input(input, { target: { value: "correction B" } });
    firstSave.resolve(undefined);
    await waitFor(() => expect(save.hasAttribute("disabled")).toBe(false));
    expect(input.value).toBe("correction B");

    await fireEvent.click(save);
    await waitFor(() => expect(api.saveCorrections).toHaveBeenCalledTimes(2));
    expect(api.saveCorrections.mock.calls[1]).toEqual([
      "session-1",
      [{ segment_index: 0, corrected_text: "correction B" }],
    ]);
  });

  it("cancels an older success clear before showing a newer failure", async () => {
    api.saveCorrections
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce(new Error("database locked again"));
    const realSetTimeout = globalThis.setTimeout;
    let successTimer: ReturnType<typeof setTimeout> | undefined;
    const timeoutSpy = vi.spyOn(globalThis, "setTimeout").mockImplementation(
      ((handler: TimerHandler, timeout?: number, ...args: unknown[]) => {
        const timer = realSetTimeout(handler, timeout, ...args);
        if (timeout === 2000) successTimer = timer;
        return timer;
      }) as typeof setTimeout
    );
    const clearSpy = vi.spyOn(globalThis, "clearTimeout");

    try {
      render(Editor);
      await fireEvent.click(
        await screen.findByRole("button", { name: /1 segment/ })
      );
      const input = (await screen.findByLabelText("Corrected")) as HTMLTextAreaElement;
      await fireEvent.input(input, { target: { value: "first correction" } });
      const save = screen.getByRole("button", { name: "Save Corrections" });
      await fireEvent.click(save);
      expect(await screen.findByText("Corrections saved")).toBeTruthy();
      expect(successTimer).toBeDefined();

      await fireEvent.input(input, { target: { value: "second correction" } });
      await fireEvent.click(save);
      expect((await screen.findByRole("alert")).textContent).toContain(
        "Save failed: database locked again"
      );
      expect(clearSpy).toHaveBeenCalledWith(successTimer);
    } finally {
      if (successTimer !== undefined) clearTimeout(successTimer);
      clearSpy.mockRestore();
      timeoutSpy.mockRestore();
    }
  });

  it("keeps a save failure visible and associated when another session is expanded", async () => {
    const pendingSave = deferred<void>();
    api.getSessions.mockResolvedValue([
      summary("session-1", 4),
      summary("session-2", 5, "2026-07-17T11:00:00Z"),
    ]);
    api.getSession.mockImplementation((id: string) =>
      Promise.resolve(detail(id, id === "session-1" ? "session A text" : "session B text"))
    );
    api.saveCorrections.mockReturnValueOnce(pendingSave.promise);
    render(Editor);

    const headers = await screen.findAllByRole("button", { name: /1 segment/ });
    await fireEvent.click(headers[0]);
    await fireEvent.input(await screen.findByLabelText("Corrected"), {
      target: { value: "session A correction" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Save Corrections" }));
    await waitFor(() => expect(api.saveCorrections).toHaveBeenCalledTimes(1));

    await fireEvent.click(
      (await screen.findAllByRole("button", { name: /1 segment/ }))[1]
    );
    expect(await screen.findByText("session B text")).toBeTruthy();
    await fireEvent.input(screen.getByLabelText("Corrected"), {
      target: { value: "session B correction" },
    });
    const sessionBSave = screen.getByRole("button", { name: "Save Corrections" });
    expect(sessionBSave.hasAttribute("disabled")).toBe(false);

    pendingSave.reject(new Error("session A database locked"));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Save failed: session A database locked");
    expect(alert.textContent).toContain("session-1");
    expect(screen.getByText("session B text")).toBeTruthy();
  });

  it("does not let another session's success erase a save failure", async () => {
    const saveA = deferred<void>();
    const saveB = deferred<void>();
    api.getSessions.mockResolvedValue([
      summary("session-1", 4),
      summary("session-2", 5, "2026-07-17T11:00:00Z"),
    ]);
    api.getSession.mockImplementation((id: string) =>
      Promise.resolve(detail(id, id === "session-1" ? "session A text" : "session B text"))
    );
    api.saveCorrections.mockImplementation((id: string) =>
      id === "session-1" ? saveA.promise : saveB.promise
    );
    render(Editor);

    await fireEvent.click(
      (await screen.findAllByRole("button", { name: /1 segment/ }))[0]
    );
    await fireEvent.input(await screen.findByLabelText("Corrected"), {
      target: { value: "session A correction" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Save Corrections" }));

    await fireEvent.click(
      (await screen.findAllByRole("button", { name: /1 segment/ }))[1]
    );
    await fireEvent.input(await screen.findByLabelText("Corrected"), {
      target: { value: "session B correction" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Save Corrections" }));
    await waitFor(() => expect(api.saveCorrections).toHaveBeenCalledTimes(2));

    saveA.reject(new Error("session A database locked"));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("session-1");
    saveB.resolve(undefined);
    expect(await screen.findByText("Corrections saved")).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain(
      "Save failed: session A database locked"
    );
  });

  it("refreshes the currently expanded session when a save supersedes an event", async () => {
    const saveB = deferred<void>();
    api.getSessions.mockResolvedValue([
      summary("session-1", 4),
      summary("session-2", 5, "2026-07-17T11:00:00Z"),
    ]);
    api.getSession.mockImplementation((id: string) =>
      Promise.resolve(detail(id, id === "session-1" ? "session A initial" : "session B initial"))
    );
    api.saveCorrections.mockReturnValueOnce(saveB.promise);
    render(Editor);

    await fireEvent.click(
      (await screen.findAllByRole("button", { name: /1 segment/ }))[1]
    );
    await fireEvent.input(await screen.findByLabelText("Corrected"), {
      target: { value: "session B correction" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Save Corrections" }));
    await fireEvent.click(
      (await screen.findAllByRole("button", { name: /1 segment/ }))[0]
    );
    expect(await screen.findByText("session A initial")).toBeTruthy();

    const eventSessions = deferred<SessionSummary[]>();
    const saveSessions = deferred<SessionSummary[]>();
    api.getSessions
      .mockReset()
      .mockReturnValueOnce(eventSessions.promise)
      .mockReturnValueOnce(saveSessions.promise);
    api.getSession.mockReset().mockResolvedValue(
      detail("session-1", "session A refreshed")
    );
    const eventRefresh = tauri.profileDataChanged!();
    saveB.resolve(undefined);
    await waitFor(() => expect(api.getSessions).toHaveBeenCalledTimes(2));

    saveSessions.resolve([
      summary("session-1", 6),
      summary("session-2", 7, "2026-07-17T11:00:00Z"),
    ]);
    await new Promise((resolve) => setTimeout(resolve, 0));
    eventSessions.resolve([summary("session-1", 99)]);
    await eventRefresh;

    expect(await screen.findByText("session A refreshed")).toBeTruthy();
    expect(api.getSession).toHaveBeenCalledWith("session-1");
  });

  it("releases event detail loading when a superseding save refresh fails", async () => {
    const pendingSave = deferred<void>();
    api.saveCorrections.mockReturnValueOnce(pendingSave.promise);
    render(Editor);

    await fireEvent.click(
      await screen.findByRole("button", { name: /1 segment/ })
    );
    await fireEvent.input(await screen.findByLabelText("Corrected"), {
      target: { value: "the cluster" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Save Corrections" }));

    const eventSessions = deferred<SessionSummary[]>();
    const saveSessions = deferred<SessionSummary[]>();
    api.getSessions
      .mockReset()
      .mockReturnValueOnce(eventSessions.promise)
      .mockReturnValueOnce(saveSessions.promise);
    const eventRefresh = tauri.profileDataChanged!();
    expect(await screen.findByText("Loading sessions...")).toBeTruthy();

    pendingSave.resolve(undefined);
    await waitFor(() => expect(api.getSessions).toHaveBeenCalledTimes(2));
    saveSessions.reject(new Error("session refresh unavailable"));
    expect(await screen.findByText("Corrections saved")).toBeTruthy();

    eventSessions.resolve([summary("session-1", 99)]);
    await eventRefresh;
    await waitFor(() =>
      expect(screen.queryByText("Loading segments...")).toBeNull()
    );
  });
});
