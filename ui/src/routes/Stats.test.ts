import { render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import StatsView from "./Stats.svelte";
import type { Stats } from "../lib/api";

const api = vi.hoisted(() => ({
  getStats: vi.fn(),
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

function stats(sessionCount: number): Stats {
  return {
    session_count: sessionCount,
    pattern_count: 0,
    auto_apply_count: 0,
  };
}

describe("Stats request ordering", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauri.profileDataChanged = undefined;
  });

  it("keeps the newer stats when an older refresh finishes last", async () => {
    const first = deferred<Stats>();
    const second = deferred<Stats>();
    let calls = 0;
    api.getStats.mockImplementation(() => {
      calls += 1;
      return calls === 1 ? first.promise : second.promise;
    });

    render(StatsView);
    await waitFor(() =>
      expect(tauri.profileDataChanged).toBeTypeOf("function")
    );
    const refresh = tauri.profileDataChanged;
    if (!refresh) {
      throw new Error("profile listener was not registered");
    }
    const pending = refresh();

    second.resolve(stats(9));
    first.resolve(stats(1));
    await pending;
    await first.promise;
    await waitFor(() => {
      expect(screen.getByText("9")).toBeTruthy();
    });
    expect(screen.queryByText("1")).toBeNull();
  });
});
