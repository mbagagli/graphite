import { createEvidenceLens } from "../domain/researchLens";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useReducer,
} from "react";
import { useLibraryPreparation } from "./useLibraryPreparation";
import { emptyPaperSelection, paperSelectionReducer } from "./paperSelection";
import { GraphHistory } from "./graphHistory";
import {
  sharedMetadata,
  mergeNeighborhoods,
  explorationRelations,
  type ExplorationResult,
} from "../services/graphExploration";
import type {
  GraphMode,
  GraphGrouping,
  GraphSession,
} from "../features/graph/graphLayout";
import type {
  GraphControls,
  GraphFocusRequest,
} from "../features/graph/GraphCanvas";

import { services } from "../services/runtime";
import type {
  GraphLayerVisibility,
  GraphNode,
  GraphRelationType,
} from "../domain/graph";
import type {
  ResearchItem,
  ResearchLens,
  ResearchSelection,
} from "../domain/research";
import type {
  AgentRunResult,
  AgentEvidence,
  NeighborhoodMode,
  RelationshipEvidence,
  SemanticRelationshipEvidence,
  WorkspacePayload,
  GraphProjection,
} from "../services/contracts";
import {
  useWorkspaceSource,
  workspaceErrorMessage,
} from "./useWorkspaceSource";

export type ViewMode = "graph" | "analytics" | "reader";

const defaultLayers: GraphLayerVisibility = {
  citation: true,
  "semantically-similar": false,
  "bibliographic-coupling": true,
  "co-cited": true,
  "shared-creator": false,
  "shared-collection": false,
  "shared-tag": false,
  "zotero-related": false,
};

interface GraphSnapshot {
  projection: GraphProjection;
  activePaperId: string | null;
  layers: GraphLayerVisibility;
  activeLensId: string | null;
  temporaryLens: ResearchLens | null;
  activeCollectionId: string | null;
  highlightedPaperIds: string[];
  scopeLabel: string;
  hiddenRelationships: number;
  similarScope: string;
  selectionConnectionsOnly: boolean;
  graphMode: GraphMode;
  graphGrouping: GraphGrouping;
  session: GraphSession;
  coverageNotice: string | null;
  seedConnections: Record<string, string[]>;
}

export function useWorkspace() {
  const evidenceRequest = useRef(0);
  const history = useRef(new GraphHistory<GraphSnapshot>());
  const graphSession = useRef<GraphSession>({
    cameras: {},
    startupPlayed: false,
  });
  const graphControls = useRef<GraphControls | null>(null);
  const [graphMode, setGraphMode] = useState<GraphMode>("3d");
  const [graphGrouping, setGraphGrouping] =
    useState<GraphGrouping>("relations");
  const [restoreSerial, setRestoreSerial] = useState(0);
  const [historyDepth, setHistoryDepth] = useState(0);
  const [explorationPending, setExplorationPending] = useState<string | null>(
    null,
  );
  const [explorationError, setExplorationError] = useState<string | null>(null);
  const [coverageNotice, setCoverageNotice] = useState<string | null>(null);
  const [seedConnections, setSeedConnections] = useState<
    Record<string, string[]>
  >({});
  const [similarScope, setSimilarScope] = useState<string>("library");
  const [selectionConnectionsOnly, setSelectionConnectionsOnly] =
    useState(false);
  const retryExploration = useRef<(() => void) | null>(null);
  const resetExplorations = useCallback(() => {
    evidenceRequest.current++;
    retryExploration.current = null;
    const hadHistory = history.current.depth > 0;
    history.current.clear();
    setHistoryDepth(0);
    setExplorationPending(null);
    setExplorationError(null);
    if (hadHistory)
      setCoverageNotice("Graph history reset because library data changed.");
    setSeedConnections({});
  }, []);
  const [focusRequest, setFocusRequest] = useState<GraphFocusRequest | null>(
    null,
  );
  const [view, setView] = useState<ViewMode>("graph");
  const [collected, dispatchSelection] = useReducer(
    paperSelectionReducer,
    emptyPaperSelection,
  );
  const [activePaperId, setActivePaperId] = useState<string | null>(null);
  const selection = useMemo(
    () => ({
      paperIds: collected.papers.map((paper) => paper.id),
      source: collected.source,
      activePaperId,
    }),
    [collected.papers, collected.source, activePaperId],
  );
  const focusPaper = useCallback((id: string | null) => {
    evidenceRequest.current++;
    setActivePaperId(id);
  }, []);
  const [layers, setLayers] = useState(defaultLayers);
  const [activeLensId, setActiveLensId] = useState<string | null>(null);
  const [highlightedPaperIds, setHighlightedPaperIds] = useState<string[]>([]);
  const [scopeLabel, setScopeLabel] = useState("Complete library");
  const [activeCollectionId, setActiveCollectionId] = useState<string | null>(
    null,
  );
  const [agentPrompt, setAgentPrompt] = useState<string | null>(null);
  const [temporaryLens, setTemporaryLens] = useState<ResearchLens | null>(null);
  const [readerContext, setReaderContext] = useState<{
    documentId: string;
    pageNumber?: number;
  } | null>(null);
  const [readerRequest, setReaderRequest] = useState<{
    serial: number;
    documentId?: string;
    pageNumber?: number;
  }>({ serial: 0 });
  const [citationIndexing, setCitationIndexing] = useState(false);
  const [semanticIndexing, setSemanticIndexing] = useState(false);
  const [relationshipEvidence, setRelationshipEvidence] = useState<
    RelationshipEvidence | SemanticRelationshipEvidence | null
  >(null);
  const [selectedExternalNode, setSelectedExternalNode] =
    useState<GraphNode | null>(null);
  const [hiddenRelationships, setHiddenRelationships] = useState(0);

  const dataReplaced = useCallback(
    (payload: WorkspacePayload) => {
      resetExplorations();
      focusPaper(null);
      const availableIds = new Set(
        payload.snapshot.items.map((item) => item.id),
      );
      setHighlightedPaperIds((current) =>
        current.filter((id) => availableIds.has(id)),
      );
    },
    [resetExplorations, focusPaper],
  );

  const advanceAfterSync = useCallback(
    async (
      payload: WorkspacePayload,
      isActive: () => boolean,
      commitData: (payload: WorkspacePayload) => void,
    ) => {
      if (!hasPendingCitationWork(payload)) return;
      if (isActive()) setCitationIndexing(true);
      try {
        const advanced = await advanceCitationQueue(payload);
        if (isActive()) commitData(advanced);
      } finally {
        if (isActive()) setCitationIndexing(false);
      }
    },
    [],
  );

  const finishManualSourceOperation = useCallback(() => {
    setCitationIndexing(false);
  }, []);

  const activeLibraryChanged = useCallback(() => {
    resetExplorations();
    setActiveCollectionId(null);
    setActiveLensId(null);
    setSimilarScope("library");
    dispatchSelection({ type: "reset" });
    focusPaper(null);
    setHighlightedPaperIds([]);
    setScopeLabel("Complete library");
  }, [resetExplorations, focusPaper]);

  const {
    data,
    setData,
    error,
    syncing,
    setSyncing,
    syncError,
    setSyncError,
    replaceData,
    syncNow,
    rebuildCache,
    setActiveLibrary,
    setSourceMode,
  } = useWorkspaceSource(services.workspace, {
    dataReplaced,
    advanceAfterSync,
    finishManualSourceOperation,
    activeLibraryChanged,
  });

  const preparation = useLibraryPreparation(data, replaceData);

  const rebuildCorpus = useCallback(async () => {
    setSyncing(true);
    setSyncError(null);
    try {
      replaceData(await services.workspace.rebuildCorpus());
    } catch (reason: unknown) {
      setSyncError(workspaceErrorMessage(reason));
    } finally {
      setSyncing(false);
    }
  }, [replaceData, setSyncError, setSyncing]);

  const papersById = useMemo(
    () =>
      new Map([
        ...collected.papers.map((item) => [item.id, item] as const),
        ...(data?.snapshot.items.map((item) => [item.id, item] as const) ?? []),
      ]),
    [data, collected.papers],
  );

  const activePaper = useMemo(() => {
    return selection.activePaperId
      ? (papersById.get(selection.activePaperId) ?? null)
      : null;
  }, [papersById, selection.activePaperId]);

  const activeLens = useMemo(
    () =>
      (temporaryLens?.id === activeLensId ? temporaryLens : null) ??
      data?.snapshot.lenses.find((lens) => lens.id === activeLensId) ??
      null,
    [activeLensId, data, temporaryLens],
  );

  const selectPaper = useCallback(
    (
      paperId: string,
      additive = false,
      source: ResearchSelection["source"] = "graph",
    ) => {
      const paper = papersById.get(paperId);
      if (!paper) return;
      dispatchSelection({
        type: additive ? "toggle" : "replace",
        paper,
        source,
      });
      focusPaper(paperId);
      if (view === "analytics") {
        setFocusRequest((previous) => ({
          id: paperId,
          serial: (previous?.serial ?? 0) + 1,
          dive: false,
        }));
      }
      setView("graph");
      setRelationshipEvidence(null);
      setSelectedExternalNode(null);
    },
    [focusPaper, papersById, view],
  );

  const addToSelection = useCallback(
    (paperId: string) => {
      const paper = papersById.get(paperId);
      if (paper) dispatchSelection({ type: "add", paper, source: "inspector" });
    },
    [papersById],
  );

  const clearFocus = useCallback(() => {
    focusPaper(null);
    setRelationshipEvidence(null);
    setSelectedExternalNode(null);
  }, [focusPaper]);

  const clearSelection = useCallback(() => {
    dispatchSelection({ type: "clear" });
    clearFocus();
  }, [clearFocus]);

  const toggleLayer = useCallback((relation: GraphRelationType) => {
    setLayers((current) => ({
      ...current,
      [relation]: !current[relation],
    }));
  }, []);

  const captureGraph = useCallback((): GraphSnapshot => {
    if (!data) throw new Error("The library is still loading.");
    graphControls.current?.capture();
    return structuredClone({
      projection: {
        graph: data.graph,
        items: data.snapshot.items,
        topics: data.snapshot.topics,
      },
      activePaperId,
      layers,
      activeLensId,
      temporaryLens,
      activeCollectionId,
      highlightedPaperIds,
      scopeLabel,
      hiddenRelationships,
      similarScope,
      selectionConnectionsOnly,
      graphMode,
      graphGrouping,
      session: graphSession.current,
      coverageNotice,
      seedConnections,
    });
  }, [
    data,
    activePaperId,
    layers,
    activeLensId,
    temporaryLens,
    activeCollectionId,
    highlightedPaperIds,
    scopeLabel,
    hiddenRelationships,
    similarScope,
    selectionConnectionsOnly,
    graphMode,
    graphGrouping,
    coverageNotice,
    seedConnections,
  ]);

  const applyProjection = useCallback(
    (projection: GraphProjection) => {
      evidenceRequest.current++;
      setData((current) =>
        current
          ? {
              ...current,
              graph: projection.graph,
              snapshot: {
                ...current.snapshot,
                items: projection.items,
                topics: projection.topics,
                loadedItemCount: projection.items.length,
              },
            }
          : current,
      );
      setRelationshipEvidence(null);
      setSelectedExternalNode(null);
      setView("graph");
    },
    [setData],
  );

  const captureGraphRef = useRef(captureGraph);
  useEffect(() => {
    captureGraphRef.current = captureGraph;
  }, [captureGraph]);

  const runExploration = useCallback(async function executeExploration<T>(
    label: string,
    load: () => Promise<T>,
    apply: (result: T) => void,
  ) {
    const token = history.current.begin();
    evidenceRequest.current++;
    const previous = captureGraphRef.current();
    retryExploration.current = () => {
      void executeExploration(label, load, apply);
    };
    setExplorationPending(label);
    setExplorationError(null);
    try {
      const result = await load();
      if (!history.current.commit(token, previous)) return;
      setCoverageNotice(null);
      setSeedConnections({});
      apply(result);
      setHistoryDepth(history.current.depth);
    } catch (reason: unknown) {
      if (history.current.isCurrent(token))
        setExplorationError(workspaceErrorMessage(reason));
    } finally {
      if (history.current.isCurrent(token)) setExplorationPending(null);
    }
  }, []);

  const goBack = useCallback(() => {
    const saved = history.current.back();
    setExplorationPending(null);
    setExplorationError(null);
    if (!saved) return;
    graphControls.current?.navigate();
    Object.keys(graphSession.current).forEach(
      (key) =>
        delete (graphSession.current as unknown as Record<string, unknown>)[
          key
        ],
    );
    Object.assign(graphSession.current, saved.session, {
      restore: true,
      startupPlayed: true,
    });
    setGraphMode(saved.graphMode);
    setGraphGrouping(saved.graphGrouping);
    setRestoreSerial((value) => value + 1);
    applyProjection(saved.projection);
    focusPaper(saved.activePaperId);
    setLayers(saved.layers);
    setActiveLensId(saved.activeLensId);
    setTemporaryLens(saved.temporaryLens);
    setActiveCollectionId(saved.activeCollectionId);
    setHighlightedPaperIds(saved.highlightedPaperIds);
    setScopeLabel(saved.scopeLabel);
    setHiddenRelationships(saved.hiddenRelationships);
    setSimilarScope(saved.similarScope);
    setSelectionConnectionsOnly(saved.selectionConnectionsOnly);
    setCoverageNotice(saved.coverageNotice);
    setSeedConnections(saved.seedConnections);
    setHistoryDepth(history.current.depth);
  }, [applyProjection, focusPaper]);

  const activateLens = useCallback(
    (
      lens: ResearchLens | null,
      temporary = false,
      options: { resetHistory?: boolean } = {},
    ) => {
      const ids = lens ? [...new Set(lens.paperIds)] : null;
      void runExploration(
        "Opening lens...",
        async () => {
          const projection = await services.graph.loadProjection({
            seedItemIds: ids ?? [],
            limit: ids ? Math.max(1, ids.length) : undefined,
            offset: 0,
          });
          if (!ids) return projection;
          const included = new Set(ids);
          return {
            ...projection,
            items: projection.items.filter((paper) => included.has(paper.id)),
            graph: {
              ...projection.graph,
              nodes: projection.graph.nodes.filter((node) =>
                included.has(node.id),
              ),
              edges: projection.graph.edges.filter(
                (edge) =>
                  included.has(edge.source) && included.has(edge.target),
              ),
              totalNodeCount: ids.length,
              hasMore: false,
            },
          };
        },
        (projection) => {
          applyProjection(projection);
          setTemporaryLens(temporary ? lens : null);
          setActiveLensId(lens?.id ?? null);
          focusPaper(null);
          setActiveCollectionId(null);
          setSimilarScope("library");
          setHighlightedPaperIds(lens?.paperIds ?? []);
          setScopeLabel(lens?.name ?? "Complete library");
          if (options.resetHistory) resetExplorations();
        },
      );
    },
    [runExploration, applyProjection, focusPaper, resetExplorations],
  );

  const activateCollection = useCallback(
    async (collectionId: string | null) => {
      if (!data) return;
      const collection = data.snapshot.collections.find(
        (c) => c.id === collectionId,
      );
      await runExploration(
        "Opening library view...",
        () =>
          services.graph.loadProjection({
            collectionId: collectionId ?? undefined,
            seedItemIds: [],
            offset: 0,
          }),
        (projection) => {
          applyProjection(projection);
          focusPaper(null);
          setHighlightedPaperIds([]);
          setActiveLensId(null);
          setActiveCollectionId(collectionId);
          setSimilarScope("library");
          setScopeLabel(collection?.name ?? "Complete library");
          setLayers(defaultLayers);
        },
      );
    },
    [data, runExploration, applyProjection, focusPaper],
  );

  const findPaper = useCallback(
    async (id: string) => {
      await runExploration(
        "Opening paper...",
        async () => {
          const projection = await services.graph.loadProjection({
            seedItemIds: [id],
            offset: 0,
          });
          if (!projection.items.some((p) => p.id === id))
            throw new Error("This paper is no longer available. Search again.");
          return projection;
        },
        (projection) => {
          applyProjection(projection);
          setActiveCollectionId(null);
          setSimilarScope("library");
          setActiveLensId(null);
          setHighlightedPaperIds([]);
          setScopeLabel("Complete library");
          focusPaper(id);
          setFocusRequest((previous) => ({
            id,
            serial: (previous?.serial ?? 0) + 1,
          }));
        },
      );
    },
    [runExploration, applyProjection, focusPaper],
  );

  const focusSearchResults = useCallback(
    async (resultIds: string[], selectedId?: string) => {
      if (selectedId) return findPaper(selectedId);
      if (!resultIds.length) return;
      await runExploration(
        "Opening search results...",
        () =>
          services.graph.loadProjection({
            seedItemIds: resultIds.slice(0, 1200),
            limit: Math.min(1200, Math.max(1, resultIds.length)),
            offset: 0,
          }),
        (projection) => {
          const included = new Set(resultIds);
          const filtered = {
            ...projection,
            items: projection.items.filter((p) => included.has(p.id)),
            graph: {
              ...projection.graph,
              nodes: projection.graph.nodes.filter((n) => included.has(n.id)),
              edges: projection.graph.edges.filter(
                (e) => included.has(e.source) && included.has(e.target),
              ),
              totalNodeCount: resultIds.length,
              hasMore: resultIds.length > 1200,
            },
          };
          applyProjection(filtered);
          focusPaper(null);
          setHighlightedPaperIds(resultIds);
          setActiveLensId(null);
          setActiveCollectionId(null);
          setSimilarScope("library");
          setScopeLabel(`Search results - ${filtered.items.length} papers`);
        },
      );
    },
    [runExploration, applyProjection, findPaper, focusPaper],
  );

  const applyNeighborhood = useCallback(
    (result: ExplorationResult, seeds: string[], label: string) => {
      applyProjection(result.projection);
      focusPaper(seeds.at(-1) ?? null);
      setHiddenRelationships(result.hiddenRelationships);
      setHighlightedPaperIds([]);
      setActiveLensId(null);
      setScopeLabel(`${label} - ${result.projection.graph.nodes.length} works`);
      setLayers((current) => ({
        ...current,
        [explorationRelations[result.mode]]: true,
      }));
      setSeedConnections(result.seedConnections);
      const excluded = result.seedCoverage.filter((seed) => seed.reason);
      setCoverageNotice(
        excluded.length
          ? `Using ${seeds.length - excluded.length} of ${seeds.length} selected papers. ` +
              excluded
                .map(
                  (seed) =>
                    `${papersById.get(seed.id)?.title ?? seed.id}: ${seed.reason}`,
                )
                .join(" ")
          : null,
      );
    },
    [applyProjection, focusPaper, papersById],
  );

  const openCitationNeighborhood = useCallback(
    async (
      itemIds: string | string[],
      mode: NeighborhoodMode,
      maxNodes = 80,
    ) => {
      const seeds = [
        ...new Set(typeof itemIds === "string" ? [itemIds] : itemIds),
      ];
      if (!seeds.length || !data) return;
      await runExploration(
        "Finding citation connections...",
        async () => {
          const parts = [];
          for (const seed of seeds)
            parts.push(
              await services.citations.neighborhood([seed], mode, 120),
            );
          const merged = mergeNeighborhoods(
            seeds,
            {
              graph: data.graph,
              items: data.snapshot.items,
              topics: data.snapshot.topics,
            },
            parts,
            mode,
            maxNodes,
          );
          for (const seed of merged.seedCoverage) {
            if (!seed.reason) continue;
            const { coverage } = await services.citations.references(
              seed.id,
              "all",
              1,
            );
            seed.reason =
              coverage.extractionStatus === "not-indexed"
                ? "References are not indexed; index this paper in Data Source."
                : "No connections for this exploration in the indexed local corpus; coverage may be incomplete.";
          }
          if (maxNodes >= 120) merged.projection.graph.hasMore = false;
          if (merged.seedCoverage.every((seed) => seed.reason))
            throw new Error(
              "No indexed connections are available for these papers. Try another citation view, or index references in Data Source.",
            );
          return merged;
        },
        (result) =>
          applyNeighborhood(
            result,
            seeds,
            `${neighborhoodLabel(mode)} - ${seeds.length} selected`,
          ),
      );
    },
    [data, runExploration, applyNeighborhood],
  );

  const openSemanticNeighborhood = useCallback(
    async (seedItemIds: string[], maxNodes = 54) => {
      if (!seedItemIds.length || !data) return;
      const collectionId = similarScope.startsWith("collection:")
        ? similarScope.slice("collection:".length)
        : undefined;
      const scope = collectionId
        ? (data.snapshot.collections.find((c) => c.id === collectionId)?.name ??
          "Collection")
        : "Whole library";
      await runExploration(
        "Finding similar papers...",
        async () => {
          const coverage = await Promise.all(
            seedItemIds.map(async (id) => ({
              id,
              coverage: await services.semantic.coverage(id),
            })),
          );
          const usable = coverage
            .filter((seed) => seed.coverage.paperStatus === "active")
            .map((seed) => seed.id);
          if (!usable.length)
            throw new Error(
              "None of the selected papers have usable similarity data. Index papers in Data Source first.",
            );
          const result = await services.semantic.neighborhood(
            usable,
            maxNodes,
            collectionId,
          );
          if (
            !result.projection.graph.edges.some(
              (edge) => edge.relation === "semantically-similar",
            )
          ) {
            throw new Error(
              "No similar papers were found in this scope. Try Whole library or a different selection.",
            );
          }
          const nodes = new Map(
            result.projection.graph.nodes.map((n) => [n.id, n]),
          );
          const items = new Map(result.projection.items.map((p) => [p.id, p]));
          for (const id of seedItemIds) {
            const node = data.graph.nodes.find((n) => n.id === id);
            const paper = papersById.get(id);
            if (node) nodes.set(id, node);
            if (paper) items.set(id, paper);
          }
          if (maxNodes >= 120) result.projection.graph.hasMore = false;
          const seedConnections: Record<string, string[]> = {};
          for (const edge of result.projection.graph.edges.filter(
            (e) => e.relation === "semantically-similar",
          )) {
            const ids = seedConnections[edge.target] ?? [];
            if (!ids.includes(edge.source)) ids.push(edge.source);
            seedConnections[edge.target] = ids;
          }
          return {
            ...result,
            projection: {
              ...result.projection,
              items: [...items.values()],
              graph: { ...result.projection.graph, nodes: [...nodes.values()] },
            },
            seedConnections,
            seedCoverage: seedItemIds.map((id) => ({
              id,
              reason: usable.includes(id)
                ? null
                : "No active paper embedding; index this paper in Data Source.",
            })),
          };
        },
        (result) =>
          applyNeighborhood(
            result,
            seedItemIds,
            `Similar to ${seedItemIds.length === 1 ? (papersById.get(seedItemIds[0])?.title ?? "selected paper") : `${seedItemIds.length} selected papers`} - ${scope}`,
          ),
      );
    },
    [data, similarScope, runExploration, papersById, applyNeighborhood],
  );

  const restoreLibraryGraph = useCallback(
    () => activateCollection(activeCollectionId),
    [activateCollection, activeCollectionId],
  );

  const selectGraphNode = useCallback(
    (node: GraphNode, additive = false) => {
      if (node.paperId) {
        selectPaper(node.paperId, additive, "graph");
      } else {
        evidenceRequest.current++;
        setSelectedExternalNode(node);
        setRelationshipEvidence(null);
      }
    },
    [selectPaper],
  );

  const inspectRelationship = useCallback(
    async (edgeId: string) => {
      const request = ++evidenceRequest.current;
      retryExploration.current = null;
      setExplorationError(null);
      setSelectedExternalNode(null);
      const edge = data?.graph.edges.find((e) => e.id === edgeId);
      if (!edge || !data) return;
      try {
        if (edge.relation === "semantically-similar") {
          const evidence = await services.semantic.evidence(edgeId);
          if (request === evidenceRequest.current)
            setRelationshipEvidence(evidence);
          return;
        }
        const citation =
          edge.relation === "citation"
            ? await services.citations.evidence(edgeId)
            : null;
        const source = papersById.get(edge.source),
          target = papersById.get(edge.target);
        let details: string[] = [];
        if (edge.relation === "shared-creator")
          details = sharedMetadata(
            source?.authors.map((a) => a.name) ?? [],
            target?.authors.map((a) => a.name) ?? [],
          );
        else if (edge.relation === "shared-tag")
          details = sharedMetadata(source?.tags ?? [], target?.tags ?? []);
        else if (edge.relation === "shared-collection")
          details = (source?.collectionIds ?? [])
            .filter((id) => target?.collectionIds.includes(id))
            .map(
              (id) =>
                data.snapshot.collections.find((c) => c.id === id)?.name ?? id,
            );
        else if (edge.relation === "zotero-related")
          details = [
            "An explicit related-item link from Zotero. This is not a citation or a similarity calculation.",
          ];
        else details = await services.citations.connectionDetails(edge);
        const summary = (id: string) => {
          const paper = papersById.get(id),
            node = data.graph.nodes.find((n) => n.id === id);
          return {
            id,
            title: paper?.title ?? node?.label ?? "External referenced work",
            firstAuthor: paper?.authors[0]?.name ?? null,
            year: paper?.year ?? null,
            venue: paper?.venue ?? null,
            doi: paper?.doi ?? null,
          };
        };
        const evidence: RelationshipEvidence = {
          edgeId,
          relation: edge.relation,
          source: summary(edge.source),
          target: summary(edge.target),
          evidenceLabel: edge.evidenceLabel ?? "Shared library metadata",
          evidence: citation?.evidence ?? [],
          details,
          directed: edge.directed,
        };
        if (request === evidenceRequest.current)
          setRelationshipEvidence(evidence);
      } catch (reason: unknown) {
        if (request === evidenceRequest.current)
          setExplorationError(workspaceErrorMessage(reason));
      }
    },
    [data, papersById],
  );

  const runCitationOperation = useCallback(
    async (operation: "index" | "rebuild" | "reresolve", itemId?: string) => {
      resetExplorations();
      setCitationIndexing(true);
      setSyncError(null);
      try {
        const payload =
          operation === "index"
            ? await services.citations.index(itemId, 48)
            : await services.citations[operation]();
        replaceData(payload);
      } catch (reason: unknown) {
        setSyncError(workspaceErrorMessage(reason));
      } finally {
        setCitationIndexing(false);
      }
    },
    [replaceData, setSyncError, resetExplorations],
  );

  const runSemanticOperation = useCallback(
    async (operation: "installModel" | "index" | "rebuild") => {
      resetExplorations();
      setSemanticIndexing(true);
      setSyncError(null);
      try {
        replaceData(await services.semantic[operation]());
      } catch (reason: unknown) {
        setSyncError(workspaceErrorMessage(reason));
      } finally {
        setSemanticIndexing(false);
      }
    },
    [replaceData, setSyncError, resetExplorations],
  );

  const openAgent = useCallback((query: string) => {
    setAgentPrompt(query);
  }, []);

  const applyAgentResult = useCallback((result: AgentRunResult) => {
    const itemIds = result.sources
      .map((source) => source.itemId)
      .filter((itemId): itemId is string => Boolean(itemId));
    setHighlightedPaperIds([...new Set(itemIds)]);
    setScopeLabel("Agent evidence");
  }, []);

  const commitLens = useCallback(
    (libraryId: string, id: string, lens: ResearchLens | null) => {
      resetExplorations();
      setData((current) => {
        if (current?.snapshot.activeLibraryId !== libraryId) return current;
        const lenses = current.snapshot.lenses.filter(
          (entry) => entry.id !== id,
        );
        if (lens) lenses.push(lens);
        lenses.sort((a, b) => a.name.localeCompare(b.name));
        return { ...current, snapshot: { ...current.snapshot, lenses } };
      });
    },
    [setData, resetExplorations],
  );

  const refreshLenses = useCallback(async () => {
    const libraryId = data?.snapshot.activeLibraryId;
    if (!libraryId) return;
    const lenses = await services.lenses.list(libraryId);
    setData((current) =>
      current?.snapshot.activeLibraryId === libraryId
        ? { ...current, snapshot: { ...current.snapshot, lenses } }
        : current,
    );
    return lenses;
  }, [data?.snapshot.activeLibraryId, setData]);

  const createTemporaryAgentLens = useCallback(
    (query: string, itemIds: string[]) => {
      const lens = createEvidenceLens(
        `temporary-agent-lens-${Date.now()}`,
        query,
        itemIds,
      );
      activateLens(lens, true);
    },
    [activateLens],
  );

  const openAgentSource = useCallback(
    (source: AgentEvidence, openInReader = false) => {
      if (!source.itemId) return;
      const paper = papersById.get(source.itemId);
      if (!paper) return;
      focusPaper(paper.id);
      if (openInReader && source.documentId) {
        const target = {
          documentId: source.documentId,
          pageNumber: source.pageStart ?? 1,
        };
        setReaderContext(target);
        setReaderRequest((previous) => ({
          ...target,
          serial: previous.serial + 1,
        }));
        setView("reader");
      } else {
        setView("graph");
      }
    },
    [papersById, focusPaper],
  );

  const openReader = useCallback(
    (paper?: ResearchItem, source?: AgentEvidence) => {
      if (paper) {
        focusPaper(paper.id);
      }
      const target = source?.documentId
        ? { documentId: source.documentId, pageNumber: source.pageStart ?? 1 }
        : null;
      setReaderContext(target);
      setReaderRequest((previous) => ({
        ...target,
        serial: previous.serial + 1,
      }));
      setView("reader");
    },
    [focusPaper],
  );

  return {
    data,
    error,
    syncing: syncing || preparation.busy,
    preparation,
    citationIndexing,
    semanticIndexing,
    syncError,
    view,
    setView,
    selection,
    activePaper,
    activeLens,
    activeCollectionId,
    layers,
    highlightedPaperIds,
    scopeLabel,
    agentPrompt,
    readerContext,
    readerRequest,
    relationshipEvidence,
    selectedExternalNode,
    hiddenRelationships,
    selectPaper,
    addToSelection,
    selectedPapers: collected.papers,
    selectionNotice: collected.notice,
    canUndoSelection: collected.undo !== null,
    toggleSelectedPaper: (paper: ResearchItem) =>
      dispatchSelection({ type: "toggle", paper, source: "search" }),
    removeSelectedPaper: (id: string) =>
      dispatchSelection({ type: "remove", id }),
    undoSelection: () => dispatchSelection({ type: "undo" }),
    clearSelection,
    clearFocus,
    toggleLayer,
    activateLens,
    activateCollection,
    focusSearchResults,
    findPaper,
    focusRequest,
    openCitationNeighborhood,
    openSemanticNeighborhood,
    explorationPending,
    explorationError,
    coverageNotice,
    seedConnections,
    similarScope,
    setSimilarScope,
    selectionConnectionsOnly,
    setSelectionConnectionsOnly,
    graphSession,
    graphControls,
    graphMode,
    graphGrouping,
    setGraphMode,
    setGraphGrouping,
    restoreSerial,
    historyDepth,
    goBack,
    retryExploration: () => retryExploration.current?.(),
    canRetryExploration: retryExploration.current !== null,
    restoreLibraryGraph,
    selectGraphNode,
    inspectRelationship,
    clearRelationshipInspection: () => {
      evidenceRequest.current++;
      setRelationshipEvidence(null);
      setSelectedExternalNode(null);
    },
    openAgent,
    consumeAgentPrompt: () => setAgentPrompt(null),
    applyAgentResult,
    createTemporaryAgentLens,
    refreshLenses,
    commitLens,
    openAgentSource,
    setReaderContext,
    openReader,
    syncNow: () => {
      resetExplorations();
      return syncNow();
    },
    rebuildCache: () => {
      resetExplorations();
      return rebuildCache();
    },
    rebuildCorpus: () => {
      resetExplorations();
      return rebuildCorpus();
    },
    indexCitations: (itemId?: string) => runCitationOperation("index", itemId),
    rebuildCitations: () => runCitationOperation("rebuild"),
    reresolveCitations: () => runCitationOperation("reresolve"),
    installSemanticModel: () => runSemanticOperation("installModel"),
    indexSemantic: () => runSemanticOperation("index"),
    rebuildSemantic: () => runSemanticOperation("rebuild"),
    cancelSemantic: async () => {
      await services.semantic.cancel();
    },
    setActiveLibrary: (id: Parameters<typeof setActiveLibrary>[0]) => {
      if (preparation.busy) return;
      resetExplorations();
      return setActiveLibrary(id);
    },
    setSourceMode: (mode: Parameters<typeof setSourceMode>[0]) => {
      if (preparation.busy) return;
      resetExplorations();
      return setSourceMode(mode);
    },
  };
}

function neighborhoodLabel(mode: NeighborhoodMode): string {
  switch (mode) {
    case "references":
      return "References from selected paper";
    case "cited-by":
      return "Cited by in the Graphite corpus";
    case "shared-references":
      return "Shared-reference neighbours";
    case "co-cited":
      return "Local-corpus co-cited works";
    case "semantic":
      return "Semantic paper neighbourhood";
    default:
      return "Local citation neighborhood";
  }
}

function hasPendingCitationWork(payload: WorkspacePayload): boolean {
  return (
    payload.citations.queuedDocuments > 0 ||
    payload.citations.queuedResolutions > 0
  );
}

async function advanceCitationQueue(
  payload: WorkspacePayload,
): Promise<WorkspacePayload> {
  if (payload.citations.queuedDocuments > 0) {
    return services.citations.index(undefined, 32);
  }
  if (payload.citations.queuedResolutions > 0) {
    return services.citations.reresolve();
  }
  return payload;
}
