import { describe, expect, it } from "vitest";
import { LatestRequest } from "./latest-request";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

describe("LatestRequest", () => {
  it("commits only the latest response when requests finish out of order", async () => {
    const requests = new LatestRequest();
    const first = deferred<string>();
    const second = deferred<string>();
    const committed: string[] = [];

    const load = async (result: Promise<string>) => {
      const request = requests.begin();
      const value = await result;
      if (requests.isCurrent(request)) committed.push(value);
    };

    const firstLoad = load(first.promise);
    const secondLoad = load(second.promise);
    second.resolve("second");
    await secondLoad;
    first.resolve("first");
    await firstLoad;

    expect(committed).toEqual(["second"]);
  });

  it("invalidates an in-flight request when the view collapses", () => {
    const requests = new LatestRequest();
    const request = requests.begin();
    requests.invalidate();
    expect(requests.isCurrent(request)).toBe(false);
  });
});
