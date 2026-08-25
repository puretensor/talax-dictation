import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App.svelte";
import type { AppConfig } from "./lib/api";

const api = vi.hoisted(() => ({
  createProfile: vi.fn(),
  getAppConfig: vi.fn(),
  getProfiles: vi.fn(),
  isModelReady: vi.fn(),
  switchProfile: vi.fn(),
}));

vi.mock("./lib/api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./lib/api")>()),
  ...api,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(vi.fn()),
}));

describe("profile selection", () => {
  const config: AppConfig = {
    hotkey: "Ctrl+Shift+Space",
    model: "small.en-q5_1",
    review_mode: "review_first",
    injection_strategy: "clipboard_only",
    active_profile: "default",
    vad_enabled: true,
    pre_roll_ms: 300,
    silence_stop_ms: 700,
  };

  beforeEach(() => {
    vi.clearAllMocks();
    api.getProfiles.mockResolvedValue(["default", "work"]);
    api.getAppConfig.mockResolvedValue(config);
    api.isModelReady.mockResolvedValue(true);
  });

  it("reports a switch failure and restores the active selection", async () => {
    api.switchProfile.mockRejectedValue(new Error("profile unavailable"));
    render(App);

    const select = (await screen.findByLabelText("Profile")) as HTMLSelectElement;
    await waitFor(() => expect(select.options).toHaveLength(2));
    await fireEvent.change(select, { target: { value: "work" } });

    expect((await screen.findByRole("alert")).textContent).toContain(
      "Switch profile failed: profile unavailable"
    );
    await waitFor(() => expect(select.value).toBe("default"));
    expect(api.switchProfile).toHaveBeenCalledWith("work");
  });

  it("updates the app shell after switching from the Profiles view", async () => {
    api.switchProfile.mockResolvedValue(undefined);
    render(App);

    const select = (await screen.findByLabelText("Profile")) as HTMLSelectElement;
    await waitFor(() => expect(select.options).toHaveLength(2));
    await fireEvent.click(screen.getByRole("button", { name: "Profiles" }));
    await fireEvent.click(await screen.findByRole("button", { name: "Switch" }));

    await waitFor(() => expect(select.value).toBe("work"));
    expect(await screen.findByText('Switched to "work"')).toBeTruthy();
    expect(api.switchProfile).toHaveBeenCalledWith("work");
    expect(api.getProfiles).toHaveBeenCalledTimes(1);
  });

  it("updates profile cards when switching from the shell selector", async () => {
    api.switchProfile.mockResolvedValue(undefined);
    render(App);

    const select = (await screen.findByLabelText("Profile")) as HTMLSelectElement;
    await waitFor(() => expect(select.options).toHaveLength(2));
    await fireEvent.click(screen.getByRole("button", { name: "Profiles" }));
    await fireEvent.change(select, { target: { value: "work" } });

    const activeBadge = await screen.findByText("Active");
    await waitFor(() =>
      expect(activeBadge.closest(".profile-card")?.textContent).toContain("work")
    );
    const defaultCard = Array.from(document.querySelectorAll(".profile-card")).find(
      (card) => card.textContent?.includes("default")
    );
    expect(defaultCard?.textContent).toContain("Switch");
  });

  it("keeps a valid shell selection when persisted profile state is stale", async () => {
    api.getAppConfig.mockResolvedValue({ ...config, active_profile: "missing" });
    render(App);

    const select = (await screen.findByLabelText("Profile")) as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe("default"));
    await fireEvent.click(screen.getByRole("button", { name: "Profiles" }));

    const activeBadge = await screen.findByText("Active");
    expect(activeBadge.closest(".profile-card")?.textContent).toContain("default");
    await waitFor(() => expect(select.value).toBe("default"));
  });

  it("updates the shell profile list after creating a profile", async () => {
    let resolveRefreshConfig!: (value: AppConfig) => void;
    const refreshConfig = new Promise<AppConfig>((resolve) => {
      resolveRefreshConfig = resolve;
    });
    api.getProfiles
      .mockResolvedValueOnce(["default", "work"])
      .mockResolvedValueOnce(["default", "work", "personal"]);
    api.getAppConfig
      .mockResolvedValueOnce(config)
      .mockReturnValueOnce(refreshConfig);
    api.createProfile.mockResolvedValue(undefined);
    render(App);

    const select = (await screen.findByLabelText("Profile")) as HTMLSelectElement;
    await fireEvent.click(screen.getByRole("button", { name: "Profiles" }));
    await fireEvent.click(await screen.findByRole("button", { name: "Create New" }));
    await fireEvent.input(screen.getByPlaceholderText("Profile name"), {
      target: { value: "personal" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => expect(select.disabled).toBe(true));
    resolveRefreshConfig(config);
    await waitFor(() => expect(select.options).toHaveLength(3));
    expect(select.disabled).toBe(false);
    expect(Array.from(select.options, (option) => option.value)).toContain("personal");
  });
});
