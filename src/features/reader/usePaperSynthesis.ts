import { useCallback, useEffect, useReducer, useRef, useState } from "react";

import type {
  AgentEnvironment,
  AgentRequest,
  AgentRunResult,
  AgentSettings,
  AgentStreamEvent,
} from "../../services/contracts";
import { services } from "../../services/runtime";

const PAPER_SYNTHESIS_QUESTION =
  "Synthesize this paper in one concise paragraph. State its research " +
  "question, approach, principal findings, and limitations. Use only " +
  "retrieved evidence from this paper; if the indexed evidence is " +
  "insufficient, say so explicitly.";

export interface PaperSynthesisState {
  running: boolean;
  stageMessage: string;
  draft: string;
  result: AgentRunResult | null;
  error: string | null;
}

export type PaperSynthesisAction =
  | { type: "reset" }
  | { type: "begin" }
  | { type: "event"; event: AgentStreamEvent }
  | { type: "resolved"; result: AgentRunResult }
  | { type: "failed"; error: string }
  | { type: "finish" };

export const INITIAL_PAPER_SYNTHESIS_STATE: PaperSynthesisState = {
  running: false,
  stageMessage: "Ready to synthesize indexed evidence",
  draft: "",
  result: null,
  error: null,
};

export function buildPaperSynthesisRequest(
  paperId: string,
  documentId: string,
  settings: AgentSettings,
): AgentRequest {
  return {
    question: PAPER_SYNTHESIS_QUESTION,
    providerKind: settings.providerKind,
    modelProfile: settings.modelProfile,
    detail: "concise",
    context: {
      currentItemId: paperId,
      currentDocumentId: documentId,
      selectedItemIds: [paperId],
      activeLensItemIds: [],
      graphNeighborhoodItemIds: [],
      removedContextKinds: [],
    },
    externalProvider:
      settings.providerKind === "openai-compatible"
        ? {
            endpoint: settings.externalEndpoint ?? "",
            model: settings.externalModel ?? "",
            contextTokens: settings.externalContextTokens ?? 8192,
          }
        : undefined,
  };
}

export function canGeneratePaperSynthesis(
  environment: AgentEnvironment | null,
): boolean {
  if (!environment) return false;
  if (environment.settings.providerKind === "openai-compatible") {
    return Boolean(
      environment.settings.externalEndpoint &&
      environment.settings.externalModel,
    );
  }
  return Boolean(
    environment.modelProfiles.find(
      (profile) => profile.id === environment.settings.modelProfile,
    )?.installed,
  );
}

export function reducePaperSynthesisState(
  state: PaperSynthesisState,
  action: PaperSynthesisAction,
): PaperSynthesisState {
  switch (action.type) {
    case "reset":
      return { ...INITIAL_PAPER_SYNTHESIS_STATE };
    case "begin":
      return {
        ...INITIAL_PAPER_SYNTHESIS_STATE,
        running: true,
        stageMessage: "Retrieving evidence from this paper",
      };
    case "event":
      return {
        ...state,
        draft:
          action.event.kind === "reset"
            ? ""
            : `${state.draft}${action.event.delta ?? ""}`,
        stageMessage: action.event.message ?? state.stageMessage,
        result: action.event.result ?? state.result,
      };
    case "resolved":
      return {
        ...state,
        draft: action.result.answerMarkdown,
        result: action.result,
      };
    case "failed":
      return { ...state, error: action.error };
    case "finish":
      return { ...state, running: false };
  }
}

export function usePaperSynthesis(paperId: string, documentId: string) {
  const [environment, setEnvironment] = useState<AgentEnvironment | null>(null);
  const [environmentError, setEnvironmentError] = useState<string | null>(null);
  const [state, dispatch] = useReducer(
    reducePaperSynthesisState,
    INITIAL_PAPER_SYNTHESIS_STATE,
    (value) => ({ ...value }),
  );
  const generation = useRef(0);

  useEffect(() => {
    let active = true;
    services.agent
      .environment()
      .then((next) => {
        if (active) setEnvironment(next);
      })
      .catch((reason: unknown) => {
        if (active) setEnvironmentError(errorMessage(reason));
      });
    return () => {
      active = false;
      generation.current += 1;
    };
  }, []);

  useEffect(() => {
    generation.current += 1;
    dispatch({ type: "reset" });
  }, [documentId, paperId]);

  const generate = useCallback(async () => {
    if (!environment || !canGeneratePaperSynthesis(environment) || !documentId)
      return;
    const currentGeneration = ++generation.current;
    dispatch({ type: "begin" });
    try {
      const result = await services.agent.run(
        buildPaperSynthesisRequest(paperId, documentId, environment.settings),
        (event) => {
          if (currentGeneration === generation.current) {
            dispatch({ type: "event", event });
          }
        },
      );
      if (currentGeneration === generation.current) {
        dispatch({ type: "resolved", result });
      }
    } catch (reason: unknown) {
      if (currentGeneration === generation.current) {
        dispatch({ type: "failed", error: errorMessage(reason) });
      }
    } finally {
      if (currentGeneration === generation.current) {
        dispatch({ type: "finish" });
      }
    }
  }, [documentId, environment, paperId]);

  return {
    ...state,
    environmentLoading: !environment && !environmentError,
    environmentError,
    available: canGeneratePaperSynthesis(environment),
    generate,
  };
}

function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}
