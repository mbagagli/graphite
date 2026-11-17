import { expect, it } from "vitest";
import { createEvidenceLens, lensMembershipChanges } from "./researchLens";

it("keeps the full research question and unique source membership for saving", () => {
  const question =
    "Which methods generalize across previously unseen seismic networks?";
  const sources = ["one", "two", "one"];
  const lens = createEvidenceLens("temporary-one", question, sources);
  expect(lens.description).toBe(question);
  expect(lens.name).toHaveLength(54);
  expect(lens.paperIds).toEqual(["one", "two"]);
  expect(lens.saved).toBe(false);
  expect(sources).toEqual(["one", "two", "one"]);
});

it("keeps a draft removal when papers are added from other searches", () => {
  expect(lensMembershipChanges(["a", "b"], ["b", "c", "d"])).toEqual({
    added: ["c", "d"],
    removed: ["a"],
  });
});

it("reports no membership change after restoring a removed paper", () => {
  expect(lensMembershipChanges(["a", "b"], ["b", "a"])).toEqual({
    added: [],
    removed: [],
  });
});

it("counts unique draft additions against an empty new Lens", () => {
  expect(lensMembershipChanges([], ["a", "b", "a"])).toEqual({
    added: ["a", "b"],
    removed: [],
  });
});
