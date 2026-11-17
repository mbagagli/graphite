import { describe, expect, it } from "vitest";

import type { AgentEnvironment } from "../../services/contracts";
import {
  buildPaperSynthesisRequest,
  canGeneratePaperSynthesis,
  INITIAL_PAPER_SYNTHESIS_STATE,
  reducePaperSynthesisState,
} from "./usePaperSynthesis";

const environment: AgentEnvironment = {
  settings: {
    providerKind: "graphite-managed",
    modelProfile: "lightweight",
    externalEndpoint: null,
    externalModel: null,
    externalContextTokens: null,
    answerDetail: "detailed",
  },
  hardware: {
    logicalCpus: 8,
    totalMemoryBytes: 16,
    availableMemoryBytes: 8,
    operatingSystem: "fixture",
    architecture: "fixture",
    accelerationBackend: "CPU",
    accelerationAvailable: false,
    recommendedProfile: "lightweight",
    note: "fixture",
  },
  modelProfiles: [
    {
      id: "lightweight",
      label: "Lightweight",
      modelIdentifier: "fixture",
      modelRevision: "fixture",
      artifactFilename: "fixture.gguf",
      artifactIdentity: "fixture",
      quantization: "fixture",
      expectedBytes: 1,
      sha256: "fixture",
      contextCapacity: 8192,
      license: "fixture",
      installed: true,
      status: "ready",
      installedBytes: 1,
      lastError: null,
    },
  ],
  runtimeName: "fixture",
  runtimeVersion: "fixture",
  trustStatement: "fixture",
  toolBudget: 5,
  contextBudgetTokens: 8192,
  rerankerStatus: "fixture",
};

describe("paper synthesis controller", () => {
  it("builds a concise current-paper request through the shared agent contract", () => {
    const request = buildPaperSynthesisRequest(
      "paper:one",
      "document:one",
      environment.settings,
    );

    expect(request.detail).toBe("concise");
    expect(request.context).toEqual({
      currentItemId: "paper:one",
      currentDocumentId: "document:one",
      selectedItemIds: ["paper:one"],
      activeLensItemIds: [],
      graphNeighborhoodItemIds: [],
      removedContextKinds: [],
    });
    expect(request.conversationId).toBeUndefined();
    expect(request.question).toContain("retrieved evidence from this paper");
  });

  it("requires an installed managed profile but permits configured loopback providers", () => {
    expect(canGeneratePaperSynthesis(environment)).toBe(true);
    expect(
      canGeneratePaperSynthesis({
        ...environment,
        modelProfiles: environment.modelProfiles.map((profile) => ({
          ...profile,
          installed: false,
        })),
      }),
    ).toBe(false);
    expect(
      canGeneratePaperSynthesis({
        ...environment,
        settings: {
          ...environment.settings,
          providerKind: "openai-compatible",
          externalEndpoint: "http://127.0.0.1:11434/v1",
          externalModel: "local-model",
        },
      }),
    ).toBe(true);
  });

  it("accumulates streamed text and preserves the grounded completed result", () => {
    const running = reducePaperSynthesisState(INITIAL_PAPER_SYNTHESIS_STATE, {
      type: "begin",
    });
    const partial = reducePaperSynthesisState(running, {
      type: "event",
      event: {
        kind: "delta",
        stage: "generation",
        message: "Writing grounded answer",
        delta: "Evidence-backed ",
        result: null,
      },
    });
    const next = reducePaperSynthesisState(partial, {
      type: "event",
      event: {
        kind: "delta",
        stage: "generation",
        message: null,
        delta: "synthesis.",
        result: null,
      },
    });

    expect(next.running).toBe(true);
    expect(next.draft).toBe("Evidence-backed synthesis.");
    expect(next.stageMessage).toBe("Writing grounded answer");
  });
});
