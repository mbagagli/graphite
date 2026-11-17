import { describe, expect, it } from "vitest";
import { GraphHistory } from "./graphHistory";
describe("graph history", () => {
  it("commits only successful current requests and ignores superseded replies", () => {
    const history = new GraphHistory<string>();
    const old = history.begin(),
      current = history.begin();
    expect(history.commit(old, "old")).toBe(false);
    expect(history.depth).toBe(0);
    expect(history.commit(current, "before")).toBe(true);
    expect(history.back()).toBe("before");
    expect(history.isCurrent(current)).toBe(false);
  });
  it("bounds memory and discards the abandoned branch after Back", () => {
    const history = new GraphHistory<number>();
    for (let n = 0; n < 25; n++) history.commit(history.begin(), n);
    expect(history.depth).toBe(20);
    expect(history.back()).toBe(24);
    history.commit(history.begin(), 100);
    expect(history.back()).toBe(100);
    expect(history.back()).toBe(23);
    const pending = history.begin();
    history.clear();
    expect(history.commit(pending, 200)).toBe(false);
    expect(history.depth).toBe(0);
  });
});
