import { AnimatePresence } from "motion/react";
import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";

import { SelectionPanel } from "../features/library/SelectionPanel";
import { LensEditor } from "../features/library/LensEditor";
import type { ResearchLens } from "../domain/research";
import { isDesktopRuntime } from "../services/runtime";
import { AgentBar } from "../features/agent/AgentBar";
import { RelationshipInspector } from "../features/citations/RelationshipInspector";
import { PaperInspector } from "../features/inspector/PaperInspector";
import { LibraryRail } from "../features/library/LibraryRail";
import { SearchSurface } from "../features/search/SearchSurface";
import { SourceConnectionNotice } from "./SourceConnectionNotice";
import { TopBar } from "./TopBar";
import { useWorkspace } from "./useWorkspace";

const AnalyticsView = lazy(() =>
  import("../features/analytics/AnalyticsView").then((module) => ({
    default: module.AnalyticsView,
  })),
);

const GraphExplorer = lazy(() =>
  import("../features/graph/GraphExplorer").then((module) => ({
    default: module.GraphExplorer,
  })),
);

const ReaderView = lazy(() =>
  import("../features/reader/ReaderView").then((module) => ({
    default: module.ReaderView,
  })),
);

export function App() {
  const workspace = useWorkspace();
  const [lensEditor, setLensEditor] = useState<{
    libraryId: string;
    lens: ResearchLens | null;
  } | null>(null);
  const [lensError, setLensError] = useState<string | null>(null);
  const [lensAskRequest, setLensAskRequest] = useState(0);
  const [searchOpen, setSearchOpen] = useState(false);
  const [selectionOpen, setSelectionOpen] = useState(false);
  const closeSearch = useCallback(() => setSearchOpen(false), []);
  const [topBarPanel, setTopBarPanel] = useState<"source" | null>(null);

  useEffect(() => {
    const handleShortcut = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setSearchOpen((current) => !current);
      }
      if (event.key === "Escape" && searchOpen) {
        setSearchOpen(false);
      }
    };
    window.addEventListener("keydown", handleShortcut);
    return () => window.removeEventListener("keydown", handleShortcut);
  }, [searchOpen]);

  const relatedPapers = useMemo(() => {
    if (!workspace.activePaper || !workspace.data) return [];
    const connectedIds = new Set(
      workspace.data.graph.edges.flatMap((edge) => {
        if (edge.source === workspace.activePaper?.id) return [edge.target];
        if (edge.target === workspace.activePaper?.id) return [edge.source];
        return [];
      }),
    );
    return workspace.data.snapshot.items
      .filter((paper) => connectedIds.has(paper.id))
      .sort((left, right) => right.importance - left.importance)
      .slice(0, 5);
  }, [workspace.activePaper, workspace.data]);

  const collectionsById = useMemo(
    () =>
      new Map(
        workspace.data?.snapshot.collections.map((collection) => [
          collection.id,
          collection.name,
        ]) ?? [],
      ),
    [workspace.data],
  );

  if (workspace.error) {
    return (
      <main className="fatal-state">
        <span>WORKSPACE ERROR</span>
        <h1>The research environment could not be prepared.</h1>
        <p>{workspace.error}</p>
      </main>
    );
  }

  if (!workspace.data) {
    return (
      <main className="loading-state">
        <div className="loading-orbit">
          <span />
          <span />
          <span />
        </div>
        <span>Opening local research workspace</span>
      </main>
    );
  }

  async function editSavedLens(lens: ResearchLens): Promise<void> {
    const libraryId = workspace.data?.snapshot.activeLibraryId;
    if (!libraryId) return;
    setLensError(null);
    try {
      const lenses = await workspace.refreshLenses();
      const current = lenses?.find((entry) => entry.id === lens.id);
      if (!current)
        throw new Error("This Lens was deleted. Choose another Lens.");
      setLensEditor({ libraryId, lens: current });
    } catch (reason) {
      setLensError(reason instanceof Error ? reason.message : String(reason));
    }
  }

  const { snapshot, graph, analytics } = workspace.data;
  const readerVisible =
    workspace.view === "reader" && workspace.activePaper !== null;

  return (
    <div className={`app-shell ${readerVisible ? "is-reader-view" : ""}`}>
      <a className="skip-link" href="#main-content">
        Skip to main content
      </a>
      <TopBar
        preparation={workspace.preparation}
        openPanel={topBarPanel}
        onPanelChange={setTopBarPanel}
        view={workspace.view}
        selectedCount={workspace.selection.paperIds.length}
        selectionOpen={selectionOpen}
        onSelectionOpen={() => setSelectionOpen((open) => !open)}
        onViewChange={(view) => {
          workspace.setView(view);
        }}
        onSearchOpen={() => setSearchOpen(true)}
        source={workspace.data.source}
        corpus={workspace.data.corpus}
        extraction={workspace.data.extraction}
        citations={workspace.data.citations}
        semantic={workspace.data.semantic}
        libraries={workspace.data.snapshot.libraries}
        syncing={workspace.syncing}
        citationIndexing={workspace.citationIndexing}
        semanticIndexing={workspace.semanticIndexing}
        syncError={workspace.syncError}
        onSync={() => void workspace.syncNow()}
        onRebuild={() => void workspace.rebuildCache()}
        onRebuildCorpus={() => void workspace.rebuildCorpus()}
        onRebuildCitations={() => void workspace.rebuildCitations()}
        onInstallSemanticModel={() => void workspace.installSemanticModel()}
        onCancelSemantic={() => void workspace.cancelSemantic()}
        onRebuildSemantic={() => void workspace.rebuildSemantic()}
        onLibraryChange={(libraryId) =>
          void workspace.setActiveLibrary(libraryId)
        }
        onModeChange={workspace.setSourceMode}
      />
      <SourceConnectionNotice
        key={`${workspace.data.source.health}:${workspace.syncError ?? workspace.data.source.lastError ?? "ready"}`}
        source={workspace.data.source}
        syncing={workspace.syncing}
        syncError={workspace.syncError}
        onRetry={() => void workspace.syncNow()}
        onUseDemo={() => workspace.setSourceMode("demo")}
      />

      <div id="main-content" className="application-content" tabIndex={-1}>
        <div
          className={`workspace-shell ${readerVisible ? "is-inactive" : ""}`}
          aria-hidden={readerVisible}
          inert={readerVisible ? true : undefined}
        >
          <LibraryRail
            paperCount={snapshot.totalItemCount}
            collections={snapshot.collections}
            lenses={snapshot.lenses}
            onEditLens={(lens) => void editSavedLens(lens)}
            lensTools={
              <div className="lens-tools">
                {lensError && <p role="alert">{lensError}</p>}
                <button
                  type="button"
                  disabled={!isDesktopRuntime || !snapshot.activeLibraryId}
                  onClick={() =>
                    setLensEditor({
                      libraryId: snapshot.activeLibraryId!,
                      lens: null,
                    })
                  }
                >
                  Save selection as Lens
                </button>
                {workspace.activeLens && (
                  <>
                    <button
                      type="button"
                      disabled={!isDesktopRuntime}
                      onClick={() => {
                        const lens = workspace.activeLens;
                        if (!lens) return;
                        if (lens.saved) void editSavedLens(lens);
                        else
                          setLensEditor({
                            libraryId: snapshot.activeLibraryId!,
                            lens,
                          });
                      }}
                    >
                      {workspace.activeLens.saved
                        ? "Edit Lens"
                        : "Save temporary Lens"}
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        setLensAskRequest((value) => value + 1);
                        workspace.openAgent(
                          workspace.activeLens?.description ||
                            "What evidence do these papers provide?",
                        );
                      }}
                    >
                      Ask about this Lens
                    </button>
                    {(workspace.activeLens.unavailablePaperIds?.length ?? 0) >
                      0 && (
                      <p>
                        {workspace.activeLens.unavailablePaperIds!.length}{" "}
                        unavailable papers retained.
                      </p>
                    )}
                  </>
                )}
                {!isDesktopRuntime && <p>Save Lenses in Graphite Desktop.</p>}
              </div>
            }
            activeCollectionId={workspace.activeCollectionId}
            activeLensId={workspace.activeLens?.id ?? null}
            onAllPapers={() => workspace.activateCollection(null)}
            onCollection={workspace.activateCollection}
            onLens={workspace.activateLens}
            provenance={snapshot.provenance}
            activeLibrary={snapshot.libraries.find(
              (library) => library.id === snapshot.activeLibraryId,
            )}
          />

          <main className="workspace-main">
            <div
              className={`workspace-view-layer workspace-view-layer--graph ${
                workspace.view === "graph" ? "is-active" : ""
              }`}
              aria-hidden={workspace.view !== "graph"}
              inert={workspace.view !== "graph" ? true : undefined}
            >
              <Suspense
                fallback={
                  <div className="view-loading">Mapping library...</div>
                }
              >
                <GraphExplorer
                  onPrepareTopics={() => setTopBarPanel("source")}
                  active={workspace.view === "graph"}
                  focusRequest={workspace.focusRequest}
                  graph={graph}
                  papers={snapshot.items}
                  topics={snapshot.topics}
                  selection={workspace.selection}
                  selectedEntityId={workspace.selectedExternalNode?.id ?? null}
                  layers={workspace.layers}
                  highlightedPaperIds={workspace.highlightedPaperIds}
                  activeLens={workspace.activeLens}
                  scopeLabel={workspace.scopeLabel}
                  onSelectNode={workspace.selectGraphNode}
                  onSelectEdge={workspace.inspectRelationship}
                  onClearSelection={() => {
                    workspace.clearSelection();
                    setSelectionOpen(false);
                  }}
                  onToggleLayer={workspace.toggleLayer}
                  onClearLens={() => workspace.activateLens(null)}
                  activePaperId={workspace.selection.activePaperId}
                  mode={workspace.graphMode}
                  grouping={workspace.graphGrouping}
                  onGroupingChange={workspace.setGraphGrouping}
                  onModeChange={workspace.setGraphMode}
                  session={workspace.graphSession}
                  controls={workspace.graphControls}
                  restoreSerial={workspace.restoreSerial}
                  exploration={{
                    selectedCount: workspace.selection.paperIds.length,
                    collections: snapshot.collections,
                    similarScope: workspace.similarScope,
                    onScopeChange: workspace.setSimilarScope,
                    semanticAvailable:
                      workspace.data.semantic.modelInstalled &&
                      workspace.data.semantic.indexedPapers > 0,
                    citationIndexed:
                      workspace.data.citations.indexedDocuments > 0,
                    pending: workspace.explorationPending,
                    error: workspace.explorationError,
                    notice: workspace.coverageNotice,
                    onRetry: workspace.retryExploration,
                    canRetry: workspace.canRetryExploration,
                    onSimilar: () =>
                      void workspace.openSemanticNeighborhood(
                        workspace.selection.paperIds,
                      ),
                    onCitations: (mode) =>
                      void workspace.openCitationNeighborhood(
                        workspace.selection.paperIds,
                        mode,
                      ),
                    canGoBack: workspace.historyDepth > 0,
                    onBack: workspace.goBack,
                    selectionConnectionsOnly:
                      workspace.selectionConnectionsOnly,
                    onSelectionConnectionsChange:
                      workspace.setSelectionConnectionsOnly,
                  }}
                />
              </Suspense>
            </div>

            <div
              className={`workspace-view-layer workspace-view-layer--analytics ${
                workspace.view === "analytics" ? "is-active" : ""
              }`}
              aria-hidden={workspace.view !== "analytics"}
              inert={workspace.view !== "analytics" ? true : undefined}
            >
              <Suspense
                fallback={<div className="view-loading">Computing view...</div>}
              >
                <AnalyticsView
                  key={`${snapshot.provenance}:${snapshot.activeLibraryId}`}
                  analytics={analytics}
                  topics={snapshot.topics}
                  items={snapshot.items}
                  onSelectPaper={(paperId) =>
                    workspace.selectPaper(paperId, false, "graph")
                  }
                  provenance={snapshot.provenance}
                  scopeLabel={workspace.scopeLabel}
                  totalItemCount={snapshot.totalItemCount}
                />
              </Suspense>
            </div>

            <AnimatePresence>
              {workspace.view === "graph" &&
                workspace.activePaper &&
                !workspace.relationshipEvidence &&
                !workspace.selectedExternalNode && (
                  <PaperInspector
                    key={workspace.activePaper.id}
                    paper={workspace.activePaper}
                    seedTitles={(
                      workspace.seedConnections[workspace.activePaper.id] ?? []
                    ).map(
                      (id) =>
                        snapshot.items.find((p) => p.id === id)?.title ?? id,
                    )}
                    collections={collectionsById}
                    selected={workspace.selection.paperIds.includes(
                      workspace.activePaper.id,
                    )}
                    relatedPapers={relatedPapers}
                    onClose={workspace.clearFocus}
                    onOpenReader={() =>
                      workspace.openReader(workspace.activePaper ?? undefined)
                    }
                    onAddToSelection={() =>
                      workspace.addToSelection(workspace.activePaper?.id ?? "")
                    }
                    onAsk={() => {
                      workspace.openAgent(
                        `Summarize the methods in ${workspace.activePaper?.title}.`,
                      );
                    }}
                    onSelectRelated={(paperId) =>
                      workspace.selectPaper(paperId, false, "graph")
                    }
                    onNavigateCitation={(paperId) =>
                      void workspace.openCitationNeighborhood(
                        paperId,
                        "citation",
                      )
                    }
                    onIndexCitations={() =>
                      workspace.indexCitations(workspace.activePaper?.id)
                    }
                    semanticAvailable={
                      workspace.data.semantic.modelInstalled &&
                      workspace.data.semantic.indexedPapers > 0
                    }
                  />
                )}
              {workspace.view === "graph" &&
                (workspace.relationshipEvidence ||
                  workspace.selectedExternalNode) && (
                  <RelationshipInspector
                    key={
                      workspace.relationshipEvidence?.edgeId ??
                      workspace.selectedExternalNode?.id
                    }
                    evidence={workspace.relationshipEvidence}
                    externalNode={workspace.selectedExternalNode}
                    onClose={workspace.clearRelationshipInspection}
                    seedTitles={(
                      workspace.seedConnections[
                        workspace.selectedExternalNode?.id ?? ""
                      ] ?? []
                    ).map(
                      (id) =>
                        snapshot.items.find((paper) => paper.id === id)
                          ?.title ?? id,
                    )}
                    onNavigatePaper={(paperId) => {
                      const node = graph.nodes.find(
                        (entry) => entry.id === paperId,
                      );
                      if (node?.entityKind === "external-work")
                        workspace.selectGraphNode(node);
                      else void workspace.findPaper(paperId);
                    }}
                  />
                )}
            </AnimatePresence>
          </main>
        </div>

        {readerVisible && workspace.activePaper && (
          <Suspense
            fallback={<div className="view-loading">Preparing reader...</div>}
          >
            <ReaderView
              key={`${workspace.activePaper.id}:${workspace.readerRequest.serial}`}
              paper={workspace.activePaper}
              topics={snapshot.topics}
              initialDocumentId={workspace.readerRequest.documentId}
              initialPageNumber={workspace.readerRequest.pageNumber}
              onContextChange={workspace.setReaderContext}
              onBack={() => workspace.setView("graph")}
              onAsk={() => {
                workspace.openAgent(
                  `What methods and evidence are central to ${workspace.activePaper?.title}?`,
                );
              }}
            />
          </Suspense>
        )}

        <AgentBar
          lensAskRequest={lensAskRequest}
          semantic={workspace.data.semantic}
          availableTextDocuments={workspace.data.corpus.fullTextCount}
          sourceMode={workspace.data.source.mode}
          onLibrarySetup={() => setTopBarPanel("source")}
          selection={workspace.selection}
          activePaper={workspace.activePaper}
          activeLens={workspace.activeLens}
          activeCollectionId={workspace.activeCollectionId}
          view={workspace.view}
          readerContext={workspace.readerContext}
          prompt={workspace.agentPrompt}
          onPromptConsumed={workspace.consumeAgentPrompt}
          onRunComplete={workspace.applyAgentResult}
          onOpenSource={workspace.openAgentSource}
          onCreateTemporaryLens={workspace.createTemporaryAgentLens}
        />
      </div>

      {selectionOpen && !searchOpen && !lensEditor && (
        <SelectionPanel
          key={`selection:${workspace.data.source.mode}:${snapshot.activeLibraryId}`}
          papers={workspace.selectedPapers}
          notice={workspace.selectionNotice}
          canUndo={workspace.canUndoSelection}
          canSave={isDesktopRuntime && Boolean(snapshot.activeLibraryId)}
          onClose={() => {
            setSelectionOpen(false);
            document.getElementById("selection-trigger")?.focus();
          }}
          onSearch={() => setSearchOpen(true)}
          onOpenPaper={(id) => {
            setSelectionOpen(false);
            void workspace.findPaper(id);
          }}
          onRemove={workspace.removeSelectedPaper}
          onClear={() => {
            workspace.clearSelection();
            setSelectionOpen(false);
          }}
          onUndo={workspace.undoSelection}
          onSave={() =>
            setLensEditor({
              libraryId: snapshot.activeLibraryId!,
              lens: null,
            })
          }
        />
      )}
      {lensEditor && lensEditor.libraryId === snapshot.activeLibraryId && (
        <LensEditor
          key={`${lensEditor.libraryId}:${lensEditor.lens?.id ?? "new"}`}
          libraryId={lensEditor.libraryId}
          lens={lensEditor.lens}
          selection={workspace.selection.paperIds}
          papers={[...workspace.selectedPapers, ...snapshot.items]}
          onClose={() => setLensEditor(null)}
          onSaved={async (lens) => {
            workspace.commitLens(lensEditor.libraryId, lens.id, lens);
            workspace.activateLens(lens, false, { resetHistory: true });
          }}
          onDeleted={async (id) => {
            workspace.commitLens(lensEditor.libraryId, id, null);
            if (workspace.activeLens?.id === id)
              workspace.activateLens(null, false, { resetHistory: true });
          }}
        />
      )}
      <SearchSurface
        key={`search:${workspace.data.source.mode}:${snapshot.activeLibraryId}`}
        selectedIds={workspace.selection.paperIds}
        onTogglePaper={workspace.toggleSelectedPaper}
        onReviewSelection={() => {
          setSearchOpen(false);
          setSelectionOpen(true);
        }}
        open={searchOpen}
        onClose={closeSearch}
        onFocusResults={(ids) => void workspace.focusSearchResults(ids)}
        onSelectPaper={(id) => {
          setSelectionOpen(false);
          void workspace.findPaper(id, { select: true });
        }}
        sourceMode={workspace.data.source.mode}
        semantic={workspace.data.semantic}
      />
    </div>
  );
}
