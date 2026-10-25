import { render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import PatternsView from "./Patterns.svelte";
import type { CorrectionPattern } from "../lib/api";

const api = vi.hoisted(() => ({
  getPatterns: vi.fn(),
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

function pattern(original: string): CorrectionPattern {
  return {
    original,
    corrected: "fixed",
    frequency: 3,
    confidence: 0.9,
    context_before: null,
    context_after: null,
  };
}

describe("Patterns request ordering", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauri.profileDataChanged = undefined;
  });

  it("keeps the newer patterns when an older refresh finishes last", async () => {
    const first = deferred<CorrectionPattern[]>();
    const second = deferred<CorrectionPattern[]>();
    let calls = 0;
    api.getPatterns.mockImplementation(() => {
      calls += 1;
      return calls === 1 ? first.promise : second.promise;
    });

    render(PatternsView);
    await waitFor(() =>
      expect(tauri.profileDataChanged).toBeTypeOf("function")
    );
    const refresh = tauri.profileDataChanged;
    if (!refresh) {
      throw new Error("profile listener was not registered");
    }
    const pending = refresh();

    second.resolve([pattern("newer-word")]);
    first.resolve([pattern("stale-word")]);
    await pending;
    await first.promise;
    await waitFor(() => {
      expect(screen.getByText("newer-word")).toBeTruthy();
    });
    expect(screen.queryByText("stale-word")).toBeNull();
  });
});
