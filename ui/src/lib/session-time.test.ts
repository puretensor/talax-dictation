import { afterEach, describe, expect, it } from "vitest";
import { formatSessionTimestamp, parseSessionTimestamp } from "./session-time";

type TzEnv = { TZ?: string };

function tzEnv(): TzEnv {
  return (globalThis as unknown as { process: { env: TzEnv } }).process.env;
}

const previousTz = tzEnv().TZ;

afterEach(() => {
  const env = tzEnv();
  if (previousTz === undefined) {
    delete env.TZ;
  } else {
    env.TZ = previousTz;
  }
});

describe("parseSessionTimestamp", () => {
  it("reads SQLite CURRENT_TIMESTAMP text as UTC", () => {
    tzEnv().TZ = "America/New_York";
    const parsed = parseSessionTimestamp("2026-07-17 10:00:00");
    expect(parsed).not.toBeNull();
    expect(parsed?.getTime()).toBe(Date.UTC(2026, 6, 17, 10, 0, 0));
  });

  it("keeps an explicit UTC ISO instant", () => {
    tzEnv().TZ = "America/New_York";
    expect(parseSessionTimestamp("2026-07-17T10:00:00Z")?.getTime()).toBe(
      Date.parse("2026-07-17T10:00:00Z"),
    );
  });

  it("returns the original text when the value is not a time", () => {
    expect(parseSessionTimestamp("not-a-time")).toBeNull();
    expect(formatSessionTimestamp("not-a-time")).toBe("not-a-time");
  });
});
