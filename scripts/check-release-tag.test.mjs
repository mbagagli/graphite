import { expect, it } from "vitest";
import { validateReleaseTag } from "./check-release-tag.mjs";

it("accepts exactly the versioned release tag", () => {
  expect(validateReleaseTag("v0.3.2", "0.3.2")).toBe("v0.3.2");
});

it.each([undefined, "main", "0.3.2", "v0.3.1", "v0.3.2-beta", "v0.3.2\n"])(
  "rejects mismatched or malformed tag %s",
  (tag) => {
    expect(() => validateReleaseTag(tag, "0.3.2")).toThrow(
      "Release tag must be v0.3.2",
    );
  },
);
