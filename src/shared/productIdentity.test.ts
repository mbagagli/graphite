import { describe, expect, it } from "vitest";

import { applicationSubtitle, assistantDisplayName } from "./productIdentity";

describe("product identity", () => {
  it("exposes the official application subtitle", () => {
    expect(applicationSubtitle).toBe("Discover Knowledge");
  });

  it("keeps assistant presentation values explicit and replaceable", () => {
    expect(assistantDisplayName.trim()).not.toHaveLength(0);
  });
});
