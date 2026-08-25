import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Profiles from "./Profiles.svelte";

const api = vi.hoisted(() => ({
  cloneProfile: vi.fn(),
  createProfile: vi.fn(),
  deleteProfile: vi.fn(),
  resetProfile: vi.fn(),
  switchProfile: vi.fn(),
}));

vi.mock("../lib/api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/api")>()),
  ...api,
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

describe("Profiles mutation failures", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("reports a create failure and preserves the form for retry", async () => {
    api.createProfile.mockRejectedValue(new Error("profile already exists"));
    render(Profiles, {
      props: {
        profiles: ["default"],
        activeProfile: "default",
        loading: false,
        onprofilechange: vi.fn(),
        onprofileschanged: vi.fn(),
      },
    });

    screen.getByText("default");
    await fireEvent.click(screen.getByRole("button", { name: "Create New" }));

    const input = screen.getByPlaceholderText("Profile name") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "default" } });
    await fireEvent.click(screen.getByRole("button", { name: "Create" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain(
      "Create profile failed: profile already exists"
    );
    expect(input.value).toBe("default");
    const dialog = screen.getByRole("dialog", { name: "Create new profile" });
    expect(dialog.contains(alert)).toBe(true);
    await waitFor(() => expect(api.createProfile).toHaveBeenCalledWith("default"));
  });

  it("reports refresh failure separately after a profile was created", async () => {
    api.createProfile.mockResolvedValue(undefined);
    const onprofileschanged = vi
      .fn()
      .mockRejectedValue(new Error("reload unavailable"));
    render(Profiles, {
      props: {
        profiles: ["default"],
        activeProfile: "default",
        loading: false,
        onprofilechange: vi.fn(),
        onprofileschanged,
      },
    });

    await fireEvent.click(screen.getByRole("button", { name: "Create New" }));
    await fireEvent.input(screen.getByPlaceholderText("Profile name"), {
      target: { value: "work" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Create" }));

    expect((await screen.findByRole("alert")).textContent).toContain(
      "Profile created, but refresh failed: reload unavailable"
    );
    expect(screen.queryByRole("dialog", { name: "Create new profile" })).toBeNull();
    expect(api.createProfile).toHaveBeenCalledTimes(1);
    expect(onprofileschanged).toHaveBeenCalledTimes(1);
  });

  it("cancels an older success clear before showing a newer failure", async () => {
    api.createProfile.mockResolvedValue(undefined);
    api.switchProfile.mockRejectedValue(new Error("profile unavailable"));
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
      render(Profiles, {
        props: {
          profiles: ["default", "work"],
          activeProfile: "default",
          loading: false,
          onprofilechange: vi.fn(),
          onprofileschanged: vi.fn().mockResolvedValue(undefined),
        },
      });

      await fireEvent.click(screen.getByRole("button", { name: "Create New" }));
      await fireEvent.input(screen.getByPlaceholderText("Profile name"), {
        target: { value: "personal" },
      });
      await fireEvent.click(screen.getByRole("button", { name: "Create" }));
      expect(await screen.findByText("Profile created")).toBeTruthy();
      expect(successTimer).toBeDefined();

      await fireEvent.click(screen.getByRole("button", { name: "Switch" }));
      expect((await screen.findByRole("alert")).textContent).toContain(
        "Switch profile failed: profile unavailable"
      );
      expect(clearSpy).toHaveBeenCalledWith(successTimer);
    } finally {
      if (successTimer !== undefined) clearTimeout(successTimer);
      clearSpy.mockRestore();
      timeoutSpy.mockRestore();
    }
  });

  it("prevents duplicate create submissions while the mutation is pending", async () => {
    const create = deferred<void>();
    api.createProfile.mockReturnValue(create.promise);
    render(Profiles, {
      props: {
        profiles: ["default"],
        activeProfile: "default",
        loading: false,
        onprofilechange: vi.fn(),
        onprofileschanged: vi.fn().mockResolvedValue(undefined),
      },
    });

    await fireEvent.click(screen.getByRole("button", { name: "Create New" }));
    await fireEvent.input(screen.getByPlaceholderText("Profile name"), {
      target: { value: "work" },
    });
    const submit = screen.getByRole("button", { name: "Create" });
    await fireEvent.click(submit);
    await fireEvent.click(submit);

    expect(api.createProfile).toHaveBeenCalledTimes(1);
    expect(submit.hasAttribute("disabled")).toBe(true);
    create.resolve(undefined);
    expect(await screen.findByText("Profile created")).toBeTruthy();
  });

  it("prevents duplicate confirmed mutations while the action is pending", async () => {
    const remove = deferred<void>();
    api.deleteProfile.mockReturnValue(remove.promise);
    render(Profiles, {
      props: {
        profiles: ["default", "work"],
        activeProfile: "default",
        loading: false,
        onprofilechange: vi.fn(),
        onprofileschanged: vi.fn().mockResolvedValue(undefined),
      },
    });

    await fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    const confirm = screen.getByRole("button", { name: "Confirm" });
    await fireEvent.click(confirm);
    await fireEvent.click(confirm);

    expect(api.deleteProfile).toHaveBeenCalledTimes(1);
    expect(confirm.hasAttribute("disabled")).toBe(true);
    remove.resolve(undefined);
    expect(await screen.findByText("Profile deleted")).toBeTruthy();
  });

  it("blocks conflicting profile actions while any mutation is pending", async () => {
    const pendingSwitch = deferred<void>();
    api.switchProfile.mockReturnValue(pendingSwitch.promise);
    const onmutationpending = vi.fn();
    render(Profiles, {
      props: {
        profiles: ["default", "work"],
        activeProfile: "default",
        loading: false,
        onprofilechange: vi.fn(),
        onprofileschanged: vi.fn().mockResolvedValue(undefined),
        onmutationpending,
      },
    });

    await fireEvent.click(screen.getByRole("button", { name: "Switch" }));
    expect(api.switchProfile).toHaveBeenCalledTimes(1);
    expect(onmutationpending).toHaveBeenCalledWith(true);
    for (const button of screen.getAllByRole("button")) {
      if (["Create New", "Switch", "Clone", "Reset", "Delete"].includes(button.textContent?.trim() ?? "")) {
        expect(button.hasAttribute("disabled")).toBe(true);
      }
    }

    pendingSwitch.resolve(undefined);
    await waitFor(() => expect(onmutationpending).toHaveBeenLastCalledWith(false));
  });
});
