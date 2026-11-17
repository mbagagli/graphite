import { describe, expect, it } from "vitest";

import type { ResearchItem, ResearchLens } from "../../domain/research";
import type {
  AgentRunResult,
  AgentSettings,
  AgentStreamEvent,
} from "../../services/contracts";
import {
  buildAgentContext,
  buildAgentContextChips,
  buildAgentRequest,
  canSubmitAgentQuestion,
  INITIAL_AGENT_RUN_STATE,
  isManagedModelUnavailable,
  reduceAgentRunState,
  type AgentContextInputs,
} from "./useAgentController";

const paper = {
  id: "paper-current",
} as ResearchItem;

const lens = {
  id: "lens-active",
  name: "Active lens",
  paperIds: ["paper-lens-a", "paper-lens-b"],
} as ResearchLens;

const managedSettings: AgentSettings = {
  providerKind: "graphite-managed",
  modelProfile: "lightweight",
  externalEndpoint: null,
  externalModel: null,
  externalContextTokens: null,
  answerDetail: "concise",
};

it("keeps broader search by default and transmits selected-only explicitly", () => {
  const inputs = contextInputs({
    selection: { paperIds: ["a", "b"], source: "graph" },
  });
  expect(buildAgentContext(inputs, []).selectedOnly).toBeUndefined();
  expect(buildAgentContext(inputs, [], true)).toMatchObject({
    selectedOnly: true,
    selectedItemIds: ["a", "b"],
  });
  expect(buildAgentContext(inputs, ["selection"], true)).toMatchObject({
    selectedOnly: true,
    selectedItemIds: [],
  });
});

function contextInputs(
  overrides: Partial<AgentContextInputs> = {},
): AgentContextInputs {
  return {
    selection: { paperIds: [], source: "graph" },
    activePaper: null,
    activeLens: null,
    activeCollectionId: null,
    view: "graph",
    readerContext: null,
    ...overrides,
  };
}

function runResult(
  runId: string,
  conversationId = "conversation-1",
): AgentRunResult {
  return {
    runId,
    conversationId,
    question: "Question",
    intent: "paper-discovery",
    answerMarkdown: `Answer ${runId}`,
    status: "complete",
    providerLabel: "Fixture",
    modelLabel: "Fixture",
    contextLabel: "Fixture context",
    contextTokens: 1,
    generationTokens: 1,
    sources: [],
    citations: [],
    limitations: [],
    actions: [],
    toolCalls: [],
    promptVersions: [],
    timings: {
      retrievalMs: 1,
      contextMs: 1,
      timeToFirstTokenMs: 1,
      generationMs: 1,
      totalMs: 4,
    },
  };
}

function streamEvent(overrides: Partial<AgentStreamEvent>): AgentStreamEvent {
  return {
    kind: "stage",
    stage: null,
    message: null,
    delta: null,
    result: null,
    ...overrides,
  };
}

describe("agent controller characterization", () => {
  it("keeps active-library fallback and exact selected context", () => {
    const fallback = contextInputs();
    expect(buildAgentContextChips(fallback, [])).toEqual([
      { kind: "library", label: "Active library" },
    ]);
    expect(buildAgentContext(fallback, [])).toEqual({
      currentItemId: undefined,
      currentDocumentId: undefined,
      currentPage: undefined,
      selectedItemIds: [],
      activeCollectionId: undefined,
      activeLensId: undefined,
      activeLensItemIds: [],
      graphNeighborhoodItemIds: [],
      removedContextKinds: [],
    });

    const scoped = contextInputs({
      selection: {
        paperIds: ["paper-current", "paper-selected"],
        source: "graph",
      },
      activePaper: paper,
      activeLens: lens,
      activeCollectionId: "collection-active",
      view: "reader",
      readerContext: { documentId: "document-current", pageNumber: 17 },
    });
    expect(buildAgentContextChips(scoped, [])).toEqual([
      { kind: "current-paper", label: "Current reader paper" },
      { kind: "reader-page", label: "Reader page 17" },
      { kind: "selection", label: "2 selected papers" },
      { kind: "lens", label: "Active lens" },
      { kind: "collection", label: "Active collection" },
    ]);
    expect(buildAgentContext(scoped, [])).toEqual({
      currentItemId: "paper-current",
      currentDocumentId: "document-current",
      currentPage: 17,
      selectedItemIds: ["paper-current", "paper-selected"],
      activeCollectionId: "collection-active",
      activeLensId: "lens-active",
      activeLensItemIds: ["paper-lens-a", "paper-lens-b"],
      graphNeighborhoodItemIds: [],
      removedContextKinds: [],
    });
  });

  it("applies removals and current context changes to the next request", () => {
    const first = contextInputs({
      selection: { paperIds: ["paper-a", "paper-b"], source: "graph" },
      activePaper: paper,
      activeLens: lens,
      activeCollectionId: "collection-active",
      view: "reader",
      readerContext: { documentId: "document-current", pageNumber: 8 },
    });
    const removed = ["current-paper", "reader-page", "lens", "collection"];
    expect(buildAgentContext(first, removed)).toEqual({
      currentItemId: undefined,
      currentDocumentId: undefined,
      currentPage: undefined,
      selectedItemIds: ["paper-a", "paper-b"],
      activeCollectionId: undefined,
      activeLensId: undefined,
      activeLensItemIds: [],
      graphNeighborhoodItemIds: [],
      removedContextKinds: removed,
    });

    const next = contextInputs({
      selection: { paperIds: ["paper-c"], source: "search" },
    });
    expect(buildAgentContext(next, []).selectedItemIds).toEqual(["paper-c"]);
  });

  it("preserves conversation reuse and external-provider request fields", () => {
    const settings: AgentSettings = {
      ...managedSettings,
      providerKind: "openai-compatible",
      externalEndpoint: "http://127.0.0.1:11434",
      externalModel: "local-model",
      externalContextTokens: null,
      answerDetail: "detailed",
    };
    const context = buildAgentContext(contextInputs(), []);

    expect(
      buildAgentRequest(
        "Question",
        "conversation-active",
        settings,
        "session-only-key",
        context,
      ),
    ).toEqual({
      question: "Question",
      conversationId: "conversation-active",
      providerKind: "openai-compatible",
      modelProfile: "lightweight",
      inferenceBackend: "auto",
      contextWindow: "medium",
      detail: "detailed",
      context,
      externalProvider: {
        endpoint: "http://127.0.0.1:11434",
        model: "local-model",
        contextTokens: 8192,
        apiKey: "session-only-key",
      },
    });
  });

  it("characterizes submit gating and explicit model readiness", () => {
    expect(canSubmitAgentQuestion("", false, managedSettings)).toBe(false);
    expect(canSubmitAgentQuestion("   ", false, managedSettings)).toBe(false);
    expect(canSubmitAgentQuestion("Question", true, managedSettings)).toBe(
      false,
    );
    expect(canSubmitAgentQuestion("Question", false, null)).toBe(false);
    expect(canSubmitAgentQuestion(" Question ", false, managedSettings)).toBe(
      true,
    );

    expect(
      isManagedModelUnavailable(
        {
          settings: managedSettings,
          hardware: {
            logicalCpus: 1,
            totalMemoryBytes: 1,
            availableMemoryBytes: 1,
            operatingSystem: "fixture",
            architecture: "fixture",
            accelerationBackend: "CPU",
            accelerationAvailable: false,
            recommendedProfile: "lightweight",
            note: "fixture",
          },
          modelProfiles: [],
          runtimeName: "fixture",
          runtimeVersion: "fixture",
          trustStatement: "fixture",
          toolBudget: 1,
          contextBudgetTokens: 1,
          rerankerStatus: "fixture",
        },
        managedSettings,
      ),
    ).toBe(true);
  });

  it("preserves streaming accumulation, completion, and error cleanup", () => {
    let state = reduceAgentRunState(INITIAL_AGENT_RUN_STATE, { type: "begin" });
    expect(state).toMatchObject({
      running: true,
      stage: "resolving-context",
      stageMessage: "Resolving intent and scope",
      draft: "",
      result: null,
      error: null,
    });

    state = reduceAgentRunState(state, {
      type: "stream-event",
      event: streamEvent({
        kind: "delta",
        stage: "generating",
        delta: "Partial ",
      }),
    });
    state = reduceAgentRunState(state, {
      type: "stream-event",
      event: streamEvent({ kind: "delta", delta: "answer" }),
    });
    expect(state.draft).toBe("Partial answer");

    state = reduceAgentRunState(state, {
      type: "failed",
      error: "Generation failed",
    });
    state = reduceAgentRunState(state, { type: "finish" });
    expect(state).toMatchObject({
      running: false,
      draft: "Partial answer",
      error: "Generation failed",
    });

    const result = runResult("run-complete");
    state = reduceAgentRunState(state, { type: "resolved", result });
    expect(state).toMatchObject({
      result,
      draft: "Answer run-complete",
      conversationId: "conversation-1",
    });
  });

  it("preserves cancellation and stale-event quirks until behavior work", () => {
    let state = reduceAgentRunState(INITIAL_AGENT_RUN_STATE, { type: "begin" });
    state = reduceAgentRunState(state, {
      type: "stream-event",
      event: streamEvent({
        kind: "cancelled",
        stage: "cancelled",
        message: "Stopped by the user",
      }),
    });
    expect(state).toMatchObject({
      running: true,
      stage: "cancelled",
      stageMessage: "Stopped by the user",
    });

    const stale = runResult("run-stale", "conversation-stale");
    state = reduceAgentRunState(state, {
      type: "stream-event",
      event: streamEvent({ kind: "complete", result: stale }),
    });
    expect(state.result).toBe(stale);
    expect(state.conversationId).toBe("conversation-stale");
    expect(reduceAgentRunState(state, { type: "finish" }).running).toBe(false);
  });

  it("preserves conversation switching, reset, and inactive deletion", () => {
    const opened = runResult("run-opened", "conversation-opened");
    let state = reduceAgentRunState(INITIAL_AGENT_RUN_STATE, {
      type: "open-conversation",
      conversationId: "conversation-opened",
      result: opened,
    });
    expect(state).toMatchObject({
      conversationId: "conversation-opened",
      result: opened,
      draft: "Answer run-opened",
    });

    expect(
      reduceAgentRunState(state, {
        type: "delete-conversation",
        conversationId: "conversation-other",
      }),
    ).toBe(state);
    state = reduceAgentRunState(state, { type: "new-conversation" });
    expect(state).toMatchObject({
      conversationId: undefined,
      result: null,
      draft: "",
    });
    expect(state.stage).toBe("ready");
  });
});

it("captures device and context choices in each generation request", () => {
  const request = buildAgentRequest(
    "Question",
    undefined,
    {
      ...managedSettings,
      inferenceBackend: "cpu",
      contextWindow: "high",
    },
    "",
    buildAgentContext(contextInputs(), []),
  );
  expect(request.inferenceBackend).toBe("cpu");
  expect(request.contextWindow).toBe("high");
});

it("transmits Lens-only explicitly and excludes unavailable members", () => {
  const inputs = contextInputs({
    activeLens: {
      ...lens,
      unavailablePaperIds: ["paper-lens-b"],
    },
  });
  expect(buildAgentContext(inputs, [], false, true)).toMatchObject({
    lensOnly: true,
    activeLensItemIds: ["paper-lens-a"],
  });
  expect(buildAgentContext(inputs, ["lens"], false, true)).toMatchObject({
    lensOnly: true,
    activeLensItemIds: [],
  });
});
