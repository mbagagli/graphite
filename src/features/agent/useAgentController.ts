import { useCallback, useEffect, useReducer, useRef, useState } from "react";

import type {
  ResearchItem,
  ResearchLens,
  ResearchSelection,
} from "../../domain/research";
import type {
  AgentContext,
  AgentEnvironment,
  AgentRequest,
  AgentRunResult,
  AgentSettings,
  AgentStreamEvent,
  ConversationSummary,
  ModelInstallEvent,
} from "../../services/contracts";
import { services } from "../../services/runtime";
import { assistantDisplayName } from "../../shared/productIdentity";

export interface ReaderAgentContext {
  documentId: string;
  pageNumber?: number;
}

export interface AgentContextInputs {
  selection: ResearchSelection;
  activePaper: ResearchItem | null;
  activeLens: ResearchLens | null;
  activeCollectionId: string | null;
  view: "graph" | "analytics" | "reader";
  readerContext: ReaderAgentContext | null;
}

export interface AgentContextChip {
  kind: string;
  label: string;
}

interface UseAgentControllerOptions extends AgentContextInputs {
  onRunComplete: (result: AgentRunResult) => void;
  onCreateTemporaryLens: (query: string, itemIds: string[]) => void;
}

export type AgentSubmitTarget = "answer" | "setup";

export interface AgentRunState {
  running: boolean;
  stage: string;
  stageMessage: string;
  draft: string;
  result: AgentRunResult | null;
  error: string | null;
  conversationId: string | undefined;
}

export type AgentRunAction =
  | { type: "begin" }
  | { type: "stream-event"; event: AgentStreamEvent }
  | { type: "resolved"; result: AgentRunResult }
  | { type: "failed"; error: string | null }
  | { type: "finish" }
  | { type: "clear-error" }
  | { type: "set-error"; error: string }
  | {
      type: "open-conversation";
      conversationId: string;
      result: AgentRunResult | null;
    }
  | { type: "new-conversation" }
  | { type: "delete-conversation"; conversationId: string };

export const INITIAL_AGENT_RUN_STATE: AgentRunState = {
  running: false,
  stage: "ready",
  stageMessage: "Ready for a grounded local question",
  draft: "",
  result: null,
  error: null,
  conversationId: undefined,
};

export function buildAgentContextChips(
  inputs: AgentContextInputs,
  removedContext: string[],
): AgentContextChip[] {
  const chips: AgentContextChip[] = [];
  if (inputs.activePaper && !removedContext.includes("current-paper")) {
    chips.push({
      kind: "current-paper",
      label:
        inputs.view === "reader" ? "Current reader paper" : "Current paper",
    });
  }
  if (
    inputs.readerContext &&
    inputs.view === "reader" &&
    !removedContext.includes("reader-page")
  ) {
    chips.push({
      kind: "reader-page",
      label: inputs.readerContext.pageNumber
        ? `Reader page ${inputs.readerContext.pageNumber}`
        : "Reader document (original)",
    });
  }
  if (
    inputs.selection.paperIds.length > 1 &&
    !removedContext.includes("selection")
  ) {
    chips.push({
      kind: "selection",
      label: `${inputs.selection.paperIds.length} selected papers`,
    });
  }
  if (inputs.activeLens && !removedContext.includes("lens")) {
    chips.push({ kind: "lens", label: inputs.activeLens.name });
  }
  if (inputs.activeCollectionId && !removedContext.includes("collection")) {
    chips.push({ kind: "collection", label: "Active collection" });
  }
  if (chips.length === 0) {
    chips.push({ kind: "library", label: "Active library" });
  }
  return chips;
}

export function buildAgentContext(
  inputs: AgentContextInputs,
  removedContext: string[],
  selectedOnly = false,
  lensOnly = false,
): AgentContext {
  return {
    ...(selectedOnly ? { selectedOnly: true } : {}),
    ...(lensOnly ? { lensOnly: true } : {}),
    currentItemId: removedContext.includes("current-paper")
      ? undefined
      : inputs.activePaper?.id,
    currentDocumentId:
      inputs.view === "reader" && !removedContext.includes("reader-page")
        ? inputs.readerContext?.documentId
        : undefined,
    currentPage:
      inputs.view === "reader" && !removedContext.includes("reader-page")
        ? inputs.readerContext?.pageNumber
        : undefined,
    selectedItemIds: removedContext.includes("selection")
      ? []
      : inputs.selection.paperIds,
    activeCollectionId: removedContext.includes("collection")
      ? undefined
      : (inputs.activeCollectionId ?? undefined),
    activeLensId: removedContext.includes("lens")
      ? undefined
      : (inputs.activeLens?.id ?? undefined),
    activeLensItemIds: removedContext.includes("lens")
      ? []
      : (inputs.activeLens?.paperIds ?? []).filter(
          (id) => !inputs.activeLens?.unavailablePaperIds?.includes(id),
        ),
    graphNeighborhoodItemIds: [],
    removedContextKinds: removedContext,
  };
}

export function buildAgentRequest(
  question: string,
  conversationId: string | undefined,
  settings: AgentSettings,
  apiKey: string,
  context: AgentContext,
): AgentRequest {
  return {
    question,
    conversationId,
    providerKind: settings.providerKind,
    modelProfile: settings.modelProfile,
    inferenceBackend: settings.inferenceBackend ?? "auto",
    contextWindow: settings.contextWindow ?? "medium",
    detail: settings.answerDetail,
    context,
    externalProvider:
      settings.providerKind === "openai-compatible"
        ? {
            endpoint: settings.externalEndpoint ?? "",
            model: settings.externalModel ?? "",
            contextTokens: settings.externalContextTokens ?? 8192,
            apiKey: apiKey || undefined,
          }
        : undefined,
  };
}

export function canSubmitAgentQuestion(
  query: string,
  running: boolean,
  settings: AgentSettings | null,
): boolean {
  return Boolean(query.trim()) && !running && settings !== null;
}

export function isManagedModelUnavailable(
  environment: AgentEnvironment | null,
  settings: AgentSettings | null,
): boolean {
  const selectedProfile = environment?.modelProfiles.find(
    (profile) => profile.id === settings?.modelProfile,
  );
  return (
    settings?.providerKind === "graphite-managed" && !selectedProfile?.installed
  );
}

export function reduceAgentRunState(
  state: AgentRunState,
  action: AgentRunAction,
): AgentRunState {
  switch (action.type) {
    case "begin":
      return {
        ...state,
        running: true,
        stage: "resolving-context",
        stageMessage: "Resolving intent and scope",
        draft: "",
        result: null,
        error: null,
      };
    case "stream-event": {
      const { event } = action;
      let draft = event.kind === "reset" ? "" : state.draft;
      if (event.delta) draft += event.delta;
      return {
        ...state,
        draft,
        stage: event.stage || state.stage,
        stageMessage: event.message || state.stageMessage,
        result: event.result ?? state.result,
        conversationId: event.result?.conversationId ?? state.conversationId,
      };
    }
    case "resolved":
      return {
        ...state,
        result: action.result,
        draft: action.result.answerMarkdown,
        conversationId: action.result.conversationId,
      };
    case "failed":
      return { ...state, error: action.error };
    case "finish":
      return { ...state, running: false };
    case "clear-error":
      return { ...state, error: null };
    case "set-error":
      return { ...state, error: action.error };
    case "open-conversation":
      return {
        ...state,
        conversationId: action.conversationId,
        result: action.result,
        draft: action.result?.answerMarkdown ?? "",
      };
    case "new-conversation":
      return {
        ...state,
        conversationId: undefined,
        result: null,
        draft: "",
      };
    case "delete-conversation":
      return state.conversationId === action.conversationId
        ? {
            ...state,
            conversationId: undefined,
            result: null,
            draft: "",
          }
        : state;
  }
}

export function useAgentController({
  selection,
  activePaper,
  activeLens,
  activeCollectionId,
  view,
  readerContext,
  onRunComplete,
  onCreateTemporaryLens,
}: UseAgentControllerOptions) {
  const [environment, setEnvironment] = useState<AgentEnvironment | null>(null);
  const [settings, setSettings] = useState<AgentSettings | null>(null);
  const settingsQueue = useRef<Promise<void>>(Promise.resolve());
  const settingsRevision = useRef(0);
  const [settingsStatus, setSettingsStatus] = useState<string | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [query, setQuery] = useState("");
  const [removedContext, setRemovedContext] = useState<string[]>([]);
  const [selectedOnly, setSelectedOnly] = useState(false);
  const [lensOnly, setLensOnly] = useState(false);
  const [conversations, setConversations] = useState<ConversationSummary[]>([]);
  const [installingProfile, setInstallingProfile] = useState<string | null>(
    null,
  );
  const [installMessage, setInstallMessage] = useState<string | null>(null);
  const [externalReady, setExternalReady] = useState(false);
  const externalCheckRevision = useRef(0);
  const [providerCheck, setProviderCheck] = useState<string | null>(null);
  const [runState, dispatchRun] = useReducer(
    reduceAgentRunState,
    INITIAL_AGENT_RUN_STATE,
    (state) => ({ ...state }),
  );

  const refreshHardware = useCallback(async () => {
    try {
      const next = await services.agent.environment();
      setEnvironment(next);
    } catch (reason: unknown) {
      dispatchRun({
        type: "set-error",
        error: `Could not refresh hardware status: ${errorMessage(reason)}`,
      });
      setEnvironment((previous) =>
        previous
          ? {
              ...previous,
              hardware: {
                ...previous.hardware,
                executionStatus: "unknown",
                accelerationBackend: "Unknown",
                accelerationAvailable: false,
                gpuDevices: [],
                note: "Hardware status could not be refreshed. Reopen the app to check it.",
              },
            }
          : previous,
      );
    }
  }, []);

  const refreshConversations = useCallback(() => {
    services.agent
      .conversations()
      .then(setConversations)
      .catch(() => setConversations([]));
  }, []);

  useEffect(() => {
    let active = true;
    services.agent
      .environment()
      .then((next) => {
        if (!active) return;
        setEnvironment(next);
        setSettings(next.settings);
      })
      .catch((reason: unknown) => {
        if (active) {
          dispatchRun({ type: "set-error", error: errorMessage(reason) });
        }
      });
    refreshConversations();
    return () => {
      active = false;
    };
  }, [refreshConversations]);

  const contextInputs: AgentContextInputs = {
    selection,
    activePaper,
    activeLens,
    activeCollectionId,
    view,
    readerContext,
  };
  const contextChips = buildAgentContextChips(contextInputs, removedContext);
  const managedUnavailable = isManagedModelUnavailable(environment, settings);

  const handleAgentEvent = useCallback(
    (event: AgentStreamEvent) => {
      dispatchRun({ type: "stream-event", event });
      if (event.result) onRunComplete(event.result);
    },
    [onRunComplete],
  );

  const submit = async (
    onTarget: (target: AgentSubmitTarget) => void,
  ): Promise<void> => {
    const question = query.trim();
    if (!canSubmitAgentQuestion(query, runState.running, settings) || !settings)
      return;
    if (
      selectedOnly &&
      (removedContext.includes("selection") || selection.paperIds.length === 0)
    ) {
      onTarget("answer");
      dispatchRun({
        type: "set-error",
        error: "Select papers or switch to broader search.",
      });
      return;
    }
    if (
      lensOnly &&
      (!activeLens ||
        removedContext.includes("lens") ||
        activeLens.paperIds.filter(
          (id) => !activeLens.unavailablePaperIds?.includes(id),
        ).length === 0 ||
        activeLens.paperIds.filter(
          (id) => !activeLens.unavailablePaperIds?.includes(id),
        ).length > 64)
    ) {
      onTarget("answer");
      dispatchRun({
        type: "set-error",
        error:
          "Ask about a Lens requires 1-64 available papers. Edit the Lens or select a smaller set.",
      });
      return;
    }
    if (managedUnavailable) {
      onTarget("setup");
      dispatchRun({
        type: "set-error",
        error: "Install the selected Graphite-managed model before generation.",
      });
      return;
    }
    dispatchRun({ type: "begin" });
    onTarget("answer");
    try {
      const run = await services.agent.run(
        buildAgentRequest(
          question,
          runState.conversationId,
          settings,
          apiKey,
          buildAgentContext(
            contextInputs,
            removedContext,
            selectedOnly,
            lensOnly,
          ),
        ),
        handleAgentEvent,
      );
      dispatchRun({ type: "resolved", result: run });
      setQuery("");
    } catch (reason: unknown) {
      const message = errorMessage(reason);
      if (!message.toLowerCase().includes("stopped by the user")) {
        dispatchRun({ type: "failed", error: message });
      }
    } finally {
      dispatchRun({ type: "finish" });
      refreshConversations();
      await refreshHardware();
    }
  };

  const installModel = async (profileId: string): Promise<void> => {
    setInstallingProfile(profileId);
    setInstallMessage("Preparing download");
    dispatchRun({ type: "clear-error" });
    try {
      const next = await services.agent.installModel(
        profileId,
        (event: ModelInstallEvent) => setInstallMessage(event.message),
      );
      setEnvironment(next);
    } catch (reason: unknown) {
      dispatchRun({ type: "set-error", error: errorMessage(reason) });
    } finally {
      setInstallingProfile(null);
    }
  };

  const persistSettings = (updated = settings): Promise<void> => {
    if (!updated) return Promise.resolve();
    if (
      updated.providerKind !== settings?.providerKind ||
      updated.externalEndpoint !== settings?.externalEndpoint ||
      updated.externalModel !== settings?.externalModel ||
      updated.externalContextTokens !== settings?.externalContextTokens
    ) {
      externalCheckRevision.current += 1;
      setExternalReady(false);
      setProviderCheck(null);
    }
    setSettings(updated);
    const revision = ++settingsRevision.current;
    setSettingsStatus("Saving...");
    setSettingsError(null);
    // Serialize writes so a slower response cannot overwrite a newer choice.
    const save = settingsQueue.current.then(async () => {
      try {
        const next = await services.agent.saveSettings(updated);
        if (revision !== settingsRevision.current) return;
        setEnvironment(next);
        setSettingsStatus("Saved locally");
      } catch (reason: unknown) {
        if (revision !== settingsRevision.current) return;
        setSettingsStatus(null);
        setSettingsError(`Changes not saved. ${errorMessage(reason)}`);
      }
    });
    settingsQueue.current = save;
    return save;
  };

  const checkExternal = async (): Promise<void> => {
    if (!settings) return;
    const revision = ++externalCheckRevision.current;
    setExternalReady(false);
    setProviderCheck("Checking loopback endpoint...");
    dispatchRun({ type: "clear-error" });
    try {
      const message = await services.agent.checkExternal({
        endpoint: settings.externalEndpoint ?? "",
        model: settings.externalModel ?? "",
        contextTokens: settings.externalContextTokens ?? 8192,
        apiKey: apiKey || undefined,
      });
      if (revision !== externalCheckRevision.current) return;
      setExternalReady(true);
      setProviderCheck(message);
    } catch (reason: unknown) {
      if (revision !== externalCheckRevision.current) return;
      setExternalReady(false);
      setProviderCheck(null);
      dispatchRun({ type: "set-error", error: errorMessage(reason) });
    }
  };

  const openConversation = async (
    conversationId: string,
    onOpened: () => void,
  ): Promise<void> => {
    dispatchRun({ type: "clear-error" });
    try {
      const conversation = await services.agent.conversation(conversationId);
      dispatchRun({
        type: "open-conversation",
        conversationId,
        result: conversation.runs.at(-1) ?? null,
      });
      onOpened();
    } catch (reason: unknown) {
      dispatchRun({ type: "set-error", error: errorMessage(reason) });
    }
  };

  const exportConversation = async (): Promise<void> => {
    if (!runState.conversationId) return;
    try {
      const markdown = await services.agent.exportMarkdown(
        runState.conversationId,
      );
      const url = URL.createObjectURL(
        new Blob([markdown], { type: "text/markdown;charset=utf-8" }),
      );
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = `graphite-conversation-${runState.conversationId.slice(-12)}.md`;
      anchor.click();
      URL.revokeObjectURL(url);
    } catch (reason: unknown) {
      dispatchRun({ type: "set-error", error: errorMessage(reason) });
    }
  };

  const deleteConversation = async (conversationId: string): Promise<void> => {
    try {
      await services.agent.deleteConversation(conversationId);
      dispatchRun({ type: "delete-conversation", conversationId });
      refreshConversations();
    } catch (reason: unknown) {
      dispatchRun({ type: "set-error", error: errorMessage(reason) });
    }
  };

  const restoreContext = useCallback(() => setRemovedContext([]), []);

  const createLens = (): void => {
    if (!runState.result) return;
    const itemIds = runState.result.sources
      .map((source) => source.itemId)
      .filter((itemId): itemId is string => Boolean(itemId));
    onCreateTemporaryLens(runState.result.question, [...new Set(itemIds)]);
  };

  return {
    environment,
    settings,
    setSettings: (updated: AgentSettings) => {
      settingsRevision.current += 1;
      externalCheckRevision.current += 1;
      setExternalReady(false);
      setProviderCheck(null);
      setSettings(updated);
      setSettingsStatus("Not saved yet");
    },
    apiKey,
    setApiKey: (value: string) => {
      externalCheckRevision.current += 1;
      setExternalReady(false);
      setProviderCheck(null);
      setApiKey(value);
    },
    query,
    setQuery,
    contextChips,
    selectedOnly,
    setSelectedOnly,
    lensOnly,
    setLensOnly,
    removedContext,
    removeContext: (kind: string) =>
      setRemovedContext((current) => [...current, kind]),
    restoreContext,
    conversations,
    installingProfile,
    installMessage,
    providerCheck,
    externalReady,
    ...runState,
    submit,
    installModel,
    persistSettings,
    settingsStatus,
    settingsError,
    checkExternal,
    openConversation,
    newConversation: () => dispatchRun({ type: "new-conversation" }),
    exportConversation,
    deleteConversation,
    createLens,
    cancel: () => services.agent.cancel(),
    unloadModel: async () => {
      await services.agent.unloadModel();
      await refreshHardware();
    },
  };
}

function errorMessage(reason: unknown): string {
  return reason instanceof Error
    ? reason.message
    : typeof reason === "string"
      ? reason
      : `${assistantDisplayName} could not complete the request.`;
}
