import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, expect, it, vi } from "vitest";
import Settings from "./Settings.svelte";

const api = vi.hoisted(() => ({
  getAvailableModels: vi.fn(),
  saveAppConfig: vi.fn(),
}));
const tauri = vi.hoisted(() => ({ downloaded: undefined as undefined | (() => void) }));
vi.mock("../lib/api", async (original) => ({
  ...(await original<typeof import("../lib/api")>()),
  ...api,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((_name: string, handler: () => void) => {
    tauri.downloaded = handler;
    return Promise.resolve(vi.fn());
  }),
}));

beforeEach(() => {
  vi.clearAllMocks();
  api.getAvailableModels.mockResolvedValue([
    { id: "small.en-q5_1", name: "Small", size_mb: 181, downloaded: false },
    { id: "tiny.en", name: "Tiny", size_mb: 75, downloaded: false },
  ]);
  api.saveAppConfig.mockResolvedValue(undefined);
});

it("refreshes downloaded models without replacing the unsaved settings draft", async () => {
  render(Settings);
  const model = (await screen.findAllByRole("combobox"))[0];
  await fireEvent.change(model, { target: { value: "tiny.en" } });
  const vad = screen.getByRole("checkbox");
  await fireEvent.click(vad);
  api.getAvailableModels.mockResolvedValue([
    { id: "small.en-q5_1", name: "Small", size_mb: 181, downloaded: false },
    { id: "tiny.en", name: "Tiny", size_mb: 75, downloaded: true },
  ]);
  tauri.downloaded!();
  await waitFor(() => expect(api.getAvailableModels).toHaveBeenCalledTimes(2));
  await screen.findByText("Downloaded", { exact: true });
  expect((screen.getAllByRole("combobox")[0] as HTMLSelectElement).value).toBe("tiny.en");
  expect((screen.getByRole("checkbox") as HTMLInputElement).checked).toBe(false);
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(api.saveAppConfig).toHaveBeenCalledWith(expect.objectContaining({
    model: "tiny.en", vad_enabled: false,
  }));
});
