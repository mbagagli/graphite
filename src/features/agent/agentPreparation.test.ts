import { describe, expect, it } from "vitest";
import { agentPreparation } from "./agentPreparation";

const ready = {
  modelInstalled: true,
  indexedChunks: 12,
  chunkedDocuments: 2,
  eligibleDocuments: 2,
  indexing: false,
};

describe("agent preparation guidance", () => {
  it("distinguishes an answer model from missing document search", () => {
    expect(
      agentPreparation(
        { ...ready, modelInstalled: false, indexedChunks: 0 },
        false,
        false,
        2,
      )?.message,
    ).toContain("search) model");
  });
  it("directs empty passage indexes to preparation without claiming corruption", () => {
    const state = agentPreparation(
      { ...ready, indexedChunks: 0 },
      false,
      false,
      2,
    );
    expect(state?.action).toBe("library");
    expect(state?.title).toContain("No document passages");
  });
  it("requests source text before indexing an empty corpus", () => {
    expect(
      agentPreparation(
        { ...ready, eligibleDocuments: 0, indexedChunks: 0 },
        false,
        false,
        0,
      )?.message,
    ).toContain("available Zotero text");
  });
  it("does not mistake uncomputed eligibility for missing synchronized text", () => {
    const state = agentPreparation(
      { ...ready, eligibleDocuments: 0, indexedChunks: 0 },
      false,
      false,
      10,
    );
    expect(state?.message).toContain("Prepare library for AI");
    expect(state?.message).not.toContain("available Zotero text");
  });
  it("directs a missing answer model to its own setup", () => {
    expect(agentPreparation(ready, true, false, 2)?.action).toBe("model");
  });
  it("requires an external connection check without demanding a managed model", () => {
    const notice = agentPreparation(ready, false, false, 2, true);
    expect(notice?.action).toBe("model");
    expect(notice?.title).toBe("Check your local AI app");
    expect(agentPreparation(ready, false, false, 2, false)).toBeNull();
  });
  it("does not present partial coverage as complete readiness", () => {
    expect(
      agentPreparation({ ...ready, eligibleDocuments: 10 }, false, false, 2)
        ?.title,
    ).toBe("2 of 10 documents prepared");
  });
  it("requires real evidence in demo workspaces too", () => {
    expect(agentPreparation(ready, false, false, 2)).toBeNull();
    expect(
      agentPreparation({ ...ready, modelInstalled: false }, true, true, 2)
        ?.action,
    ).toBe("library");
    expect(agentPreparation(ready, false, true, 0)?.message).toContain(
      "offline ZIP",
    );
  });
});
