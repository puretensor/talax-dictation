import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("vite.config.ts envPrefix", () => {
  it("does not use the broad TAURI_ prefix that leaks updater keys", () => {
    const src = readFileSync("./vite.config.ts", "utf8");
    expect(src).not.toMatch(/envPrefix:\s*\[[^\]]*["']TAURI_["']/);
    expect(src).toMatch(/["']TAURI_ENV_\*["']/);
    expect(src).toMatch(/TAURI_ENV_DEBUG/);
    expect(src).not.toMatch(/process\.env\.TAURI_DEBUG\b/);
  });
});
