import {
  BarChart3,
  ListChecks,
  Command,
  Database,
  Download,
  Network,
  RefreshCw,
  Search,
  X,
  Unlink,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { BrandMark } from "../components/BrandMark";
import { ThemeControl } from "../components/ThemeControl";
import type { ViewMode } from "./useWorkspace";
import type { LibraryDescriptor } from "../domain/research";
import type {
  CitationDiagnostics,
  CorpusDiagnostics,
  ExtractionDiagnostics,
  SourceDiagnostics,
  SemanticDiagnostics,
} from "../services/contracts";
import { isDesktopRuntime } from "../services/runtime";
import { applicationSubtitle } from "../shared/productIdentity";
import { LibraryPreparationControl } from "./LibraryPreparationControl";
import type { LibraryPreparationController } from "./useLibraryPreparation";
import { demoManifest } from "../adapters/demo/demoData";
import { AboutDialog, HelpDialog } from "./HelpDialog";

interface TopBarProps {
  openPanel: "source" | null;
  onPanelChange: (panel: "source" | null) => void;
  view: ViewMode;
  selectedCount: number;
  selectionOpen: boolean;
  onSelectionOpen: () => void;
  onViewChange: (view: ViewMode) => void;
  onSearchOpen: () => void;
  source: SourceDiagnostics;
  corpus: CorpusDiagnostics;
  extraction: ExtractionDiagnostics;
  citations: CitationDiagnostics;
  semantic: SemanticDiagnostics;
  libraries: LibraryDescriptor[];
  syncing: boolean;
  citationIndexing: boolean;
  semanticIndexing: boolean;
  syncError: string | null;
  onSync: () => void;
  onRebuild: () => void;
  onRebuildCorpus: () => void;
  onRebuildCitations: () => void;
  onInstallSemanticModel: () => void;
  onCancelSemantic: () => void;
  onRebuildSemantic: () => void;
  onLibraryChange: (libraryId: string) => void;
  onModeChange: (mode: "demo" | "zotero") => void;
  preparation: LibraryPreparationController;
}

export function TopBar({
  openPanel,
  onPanelChange,
  view,
  selectedCount,
  selectionOpen,
  onSelectionOpen,
  onViewChange,
  onSearchOpen,
  source,
  corpus,
  extraction,
  citations,
  semantic,
  libraries,
  syncing: sourceSyncing,
  citationIndexing,
  semanticIndexing,
  syncError,
  onSync,
  onRebuild,
  onRebuildCorpus,
  onRebuildCitations,
  onInstallSemanticModel,
  onCancelSemantic,
  onRebuildSemantic,
  onLibraryChange,
  onModeChange,
  preparation,
}: TopBarProps) {
  const [informationPanel, setInformationPanel] = useState<
    "help" | "about" | null
  >(null);
  const syncing = sourceSyncing || preparation.busy;
  const sourceControlRef = useRef<HTMLDivElement>(null);
  const sourcePopoverRef = useRef<HTMLElement>(null);
  const sourceOpen = openPanel === "source";
  const sourceLabel = syncing
    ? source.mode === "demo"
      ? "Preparing demo"
      : "Syncing Zotero"
    : source.mode === "demo"
      ? "Demo source"
      : source.health === "offline-cache"
        ? "Cached offline"
        : source.health === "ready"
          ? "Zotero ready"
          : "Connect Zotero";

  useEffect(() => {
    const popover = sourcePopoverRef.current;
    if (!sourceOpen || !popover) return;
    const position = () => {
      const trigger = sourceControlRef.current?.getBoundingClientRect();
      if (!trigger) return;
      popover.style.top = `${trigger.bottom + 8}px`;
      popover.style.right = `${Math.max(8, innerWidth - trigger.right)}px`;
    };
    position();
    popover.showPopover();
    popover.focus();
    window.addEventListener("resize", position);
    return () => window.removeEventListener("resize", position);
  }, [sourceOpen]);

  return (
    <header className="top-bar">
      <div className="brand-lockup">
        <BrandMark />
        <div>
          <strong>Graphite</strong>
          <span>{applicationSubtitle}</span>
        </div>
      </div>

      <nav className="view-switcher" aria-label="Primary workspace views">
        <button
          type="button"
          className={view === "graph" ? "is-active" : ""}
          onClick={() => onViewChange("graph")}
        >
          <Network size={15} />
          Explorer
        </button>
        <button
          type="button"
          className={view === "analytics" ? "is-active" : ""}
          onClick={() => onViewChange("analytics")}
        >
          <BarChart3 size={15} />
          Analytics
        </button>
      </nav>

      <div className="top-actions">
        <button
          id="selection-trigger"
          type="button"
          className="selection-readout"
          onClick={onSelectionOpen}
          aria-expanded={selectionOpen}
          aria-controls="paper-selection"
          aria-label={`Review selection (${selectedCount} papers)`}
        >
          <ListChecks size={15} />
          {selectedCount} selected
        </button>
        <button type="button" className="search-trigger" onClick={onSearchOpen}>
          <Search size={15} />
          <span>Search library</span>
          <kbd>
            <Command size={11} />K
          </kbd>
        </button>
        <div className="source-control" ref={sourceControlRef}>
          <button
            type="button"
            className={`source-trigger source-trigger--${source.health}`}
            onClick={() =>
              onPanelChange(openPanel === "source" ? null : "source")
            }
            aria-expanded={sourceOpen}
            aria-controls="source-status-popover"
          >
            {source.health === "offline-cache" ? (
              <Unlink size={14} />
            ) : (
              <Database size={14} />
            )}
            <span>{sourceLabel}</span>
            <i />
          </button>
          {sourceOpen && (
            <section
              ref={sourcePopoverRef}
              id="source-status-popover"
              className="source-popover"
              popover="auto"
              tabIndex={-1}
              onToggle={(event) => {
                if (event.newState !== "closed") return;
                onPanelChange(null);
                if (
                  document.activeElement === document.body ||
                  event.currentTarget.contains(document.activeElement)
                ) {
                  sourceControlRef.current?.querySelector("button")?.focus();
                }
              }}
              aria-label="Data source status"
            >
              <header>
                <div>
                  <span className="eyebrow">DATA SOURCE</span>
                  <strong>
                    {source.mode === "demo"
                      ? `${demoManifest.papers.length} real seismology papers`
                      : "Zotero Local API"}
                  </strong>
                </div>
                <button
                  type="button"
                  className="source-popover-close"
                  aria-label="Close library setup"
                  popoverTarget="source-status-popover"
                  popoverTargetAction="hide"
                >
                  <X size={16} />
                </button>
              </header>

              {isDesktopRuntime && (
                <div className="source-actions source-switch">
                  <button
                    type="button"
                    disabled={preparation.busy}
                    onClick={() =>
                      onModeChange(source.mode === "demo" ? "zotero" : "demo")
                    }
                  >
                    {source.mode === "demo" ? "Use Zotero" : "Use demo data"}
                  </button>
                </div>
              )}

              <LibraryPreparationControl
                preparation={preparation}
                demo={source.mode === "demo"}
                desktop={isDesktopRuntime}
                embeddingInstalled={semantic.modelInstalled}
                libraryAvailable={Boolean(source.activeLibraryId)}
                disabled={sourceSyncing || citationIndexing || semanticIndexing}
              />
              {(source.mode === "zotero" || isDesktopRuntime) && (
                <>
                  {source.mode === "zotero" && (
                    <label>
                      <span>LIBRARY</span>
                      <select
                        name="active-zotero-library"
                        value={source.activeLibraryId ?? ""}
                        onChange={(event) =>
                          onLibraryChange(event.target.value)
                        }
                        disabled={libraries.length === 0 || syncing}
                      >
                        {libraries.length === 0 && (
                          <option value="">Not discovered</option>
                        )}
                        {libraries.map((library) => (
                          <option key={library.id} value={library.id}>
                            {library.name} ({library.itemCount})
                          </option>
                        ))}
                      </select>
                    </label>
                  )}
                  <dl>
                    {source.mode === "zotero" && (
                      <>
                        <div>
                          <dt>Connection</dt>
                          <dd>
                            {syncing
                              ? "synchronizing"
                              : source.connectionStatus}
                          </dd>
                        </div>
                        <div>
                          <dt>API / schema</dt>
                          <dd>
                            {source.apiVersion ?? "-"} /{" "}
                            {source.schemaVersion ?? "-"}
                          </dd>
                        </div>
                        <div>
                          <dt>Server identity</dt>
                          <dd title={source.serverId ?? "Unavailable"}>
                            {source.serverId?.slice(0, 12) ?? "-"}
                          </dd>
                        </div>
                        <div>
                          <dt>Last successful sync</dt>
                          <dd>{formatSyncTime(source.lastSyncedAt)}</dd>
                        </div>
                        {source.syncSummary && (
                          <div>
                            <dt>Last run</dt>
                            <dd>
                              {source.syncSummary.objectsChanged} changed /{" "}
                              {source.syncSummary.objectsDeleted} deleted
                              {source.syncSummary.normalizationWarnings > 0
                                ? ` / ${source.syncSummary.normalizationWarnings} warnings`
                                : ""}
                            </dd>
                          </div>
                        )}
                      </>
                    )}
                    <div>
                      <dt>Document corpus</dt>
                      <dd>{corpus.status}</dd>
                    </div>
                    <div>
                      <dt>Documents discovered</dt>
                      <dd>{corpus.documentCount.toLocaleString()}</dd>
                    </div>
                    <div>
                      <dt>Documents with text</dt>
                      <dd>
                        {(
                          corpus.fullTextCount + corpus.partialTextCount
                        ).toLocaleString()}{" "}
                        ({corpus.fullTextCount} full / {corpus.partialTextCount}{" "}
                        partial)
                      </dd>
                    </div>
                    <div>
                      <dt>PDF resolution</dt>
                      <dd>
                        {corpus.resolvedPdfCount} / {corpus.pdfCount}
                      </dd>
                    </div>
                    <ExtractionDiagnosticRows extraction={extraction} />
                    <div className="source-diagnostic-divider">
                      <dt>Citation index</dt>
                      <dd>
                        {citationIndexing ? "indexing" : citations.status}
                      </dd>
                    </div>
                    <div>
                      <dt>Reference coverage</dt>
                      <dd>
                        {citations.indexedDocuments} /{" "}
                        {citations.eligibleDocuments} documents
                      </dd>
                    </div>
                    <div>
                      <dt>Bibliography entries</dt>
                      <dd>
                        {citations.referencesExtracted.toLocaleString()} /{" "}
                        {citations.referencesWithDoi.toLocaleString()} with DOI
                      </dd>
                    </div>
                    <div>
                      <dt>Resolution</dt>
                      <dd>
                        {citations.resolvedLocal} local /{" "}
                        {citations.externalWorks} external /{" "}
                        {citations.ambiguousReferences} review
                      </dd>
                    </div>
                    <div>
                      <dt>Local citation edges</dt>
                      <dd>
                        {citations.citationEdges.toLocaleString()} /{" "}
                        {citations.queuedResolutions.toLocaleString()} queued
                        resolutions
                      </dd>
                    </div>
                    <SemanticDiagnosticRows
                      semantic={semantic}
                      indexing={semanticIndexing}
                    />
                  </dl>
                  {[
                    ...new Set([
                      syncError,
                      source.lastError,
                      corpus.lastError,
                      citations.lastError,
                      semantic.lastError,
                    ]),
                  ].map(
                    (message) =>
                      message && (
                        <p className="source-error" key={message}>
                          {message}
                        </p>
                      ),
                  )}
                  {source.sourceChanged && (
                    <p className="source-notice">
                      A different Zotero source was detected. Its cache is kept
                      in a separate partition.
                    </p>
                  )}
                  <div className="source-actions">
                    {source.mode === "zotero" && (
                      <>
                        <button
                          type="button"
                          onClick={onSync}
                          disabled={syncing}
                        >
                          <RefreshCw
                            size={13}
                            className={syncing ? "is-spinning" : ""}
                          />
                          {syncing ? "Syncing" : "Sync now"}
                        </button>
                        <button
                          type="button"
                          onClick={() => {
                            if (
                              window.confirm(
                                "Rebuild Graphite's derived Zotero cache? Zotero itself will not be changed.",
                              )
                            ) {
                              onRebuild();
                            }
                          }}
                          disabled={syncing}
                        >
                          Rebuild cache
                        </button>
                      </>
                    )}
                    <button
                      type="button"
                      onClick={onRebuildCorpus}
                      disabled={syncing}
                      title="Delete and regenerate Graphite's document text index only"
                    >
                      Rebuild text index
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        if (
                          window.confirm(
                            "Rebuild Graphite's derived citation index? Zotero, PDFs, and compatible manual decisions remain untouched.",
                          )
                        ) {
                          onRebuildCitations();
                        }
                      }}
                      disabled={syncing || citationIndexing}
                    >
                      Rebuild citation index
                    </button>
                    {!semantic.modelInstalled ? (
                      <button
                        type="button"
                        onClick={onInstallSemanticModel}
                        disabled={syncing || semanticIndexing}
                        title="Explicitly download the pinned local ONNX model. No library content is uploaded."
                      >
                        <Download size={13} />
                        Install embedding model (
                        {formatBytes(semantic.modelDownloadBytes)})
                      </button>
                    ) : semanticIndexing ? (
                      <button type="button" onClick={onCancelSemantic}>
                        Cancel semantic indexing
                      </button>
                    ) : null}
                    {semantic.modelInstalled && (
                      <button
                        type="button"
                        onClick={() => {
                          if (
                            window.confirm(
                              "Rebuild Graphite's derived semantic index? Zotero, PDFs, and the downloaded model remain untouched.",
                            )
                          ) {
                            onRebuildSemantic();
                          }
                        }}
                        disabled={syncing || semanticIndexing}
                      >
                        Rebuild semantic index
                      </button>
                    )}
                  </div>
                  <p className="source-privacy-note">
                    Embeddings and retrieval stay on this computer. Model
                    installation downloads only the pinned model artifacts;
                    Zotero metadata and document text are never uploaded.
                  </p>
                </>
              )}

              <footer>
                <span>
                  {isDesktopRuntime
                    ? "Local application database"
                    : "Real metadata preview"}
                </span>
              </footer>
            </section>
          )}
        </div>
        <button
          type="button"
          className="top-help-button"
          aria-haspopup="dialog"
          onClick={() => {
            onPanelChange(null);
            setInformationPanel("help");
          }}
        >
          Help
        </button>
        <button
          type="button"
          className="top-help-button"
          aria-haspopup="dialog"
          onClick={() => {
            onPanelChange(null);
            setInformationPanel("about");
          }}
        >
          About
        </button>
        <ThemeControl />
      </div>
      {informationPanel === "help" && (
        <HelpDialog onClose={() => setInformationPanel(null)} />
      )}
      {informationPanel === "about" && (
        <AboutDialog onClose={() => setInformationPanel(null)} />
      )}
    </header>
  );
}

function ExtractionDiagnosticRows({
  extraction,
}: {
  extraction: ExtractionDiagnostics;
}) {
  const provider = extraction.activeProviderProfile ?? "not selected";
  return (
    <>
      <div className="source-diagnostic-divider">
        <dt>Canonical extraction</dt>
        <dd>{extraction.status}</dd>
      </div>
      <div>
        <dt>Extraction quality</dt>
        <dd>
          {extraction.good} good / {extraction.degraded} degraded /{" "}
          {extraction.failed} failed
        </dd>
      </div>
      <div>
        <dt>Backfill</dt>
        <dd>
          {extraction.pending} pending / {extraction.fallback} fallback
        </dd>
      </div>
      <div>
        <dt>Routing outcomes</dt>
        <dd>
          {extraction.unsupported} unsupported / {extraction.unavailable}{" "}
          unavailable
        </dd>
      </div>
      <div>
        <dt>Extraction profile</dt>
        <dd title={provider}>{provider}</dd>
      </div>
      <div>
        <dt>Policy profiles</dt>
        <dd
          title={`${extraction.activeRoutingProfile} / ${extraction.activeQualityProfile}`}
        >
          {extraction.activeRoutingProfile} / {extraction.activeQualityProfile}
        </dd>
      </div>
      <div className="source-diagnostic-divider">
        <dt>PDF shadow pilot</dt>
        <dd>
          {extraction.shadow.enabled
            ? extraction.shadow.running > 0
              ? "running"
              : "enabled"
            : extraction.shadow.availability}
        </dd>
      </div>
      {extraction.shadow.enabled && (
        <>
          <div>
            <dt>Shadow queue</dt>
            <dd>
              {extraction.shadow.pending} pending / {extraction.shadow.running}{" "}
              running / {extraction.shadow.cancelled} cancelled
            </dd>
          </div>
          <div>
            <dt>Shadow outcomes</dt>
            <dd>
              {extraction.shadow.completed} completed /{" "}
              {extraction.shadow.failed} failed /{" "}
              {extraction.shadow.unavailable} unavailable
            </dd>
          </div>
          <div>
            <dt>Shadow quality</dt>
            <dd>
              {extraction.shadow.good} good / {extraction.shadow.degraded}{" "}
              degraded / {extraction.shadow.qualityFailed} quality failed
            </dd>
          </div>
          <div>
            <dt>Shadow routing</dt>
            <dd>
              {extraction.shadow.fallbackRoutes} OCR fallback /{" "}
              {extraction.shadow.comparisonDiscrepancies} discrepancy
            </dd>
          </div>
          <div>
            <dt>Shadow profile</dt>
            <dd title={extraction.shadow.routingProfile}>
              {extraction.shadow.routingProfile}
            </dd>
          </div>
          {extraction.shadow.lastProviderProfile && (
            <div>
              <dt>Latest shadow attempt</dt>
              <dd title={extraction.shadow.lastRoutingReason ?? undefined}>
                {extraction.shadow.lastProviderProfile}
                {extraction.shadow.lastDurationMs !== null
                  ? ` / ${extraction.shadow.lastDurationMs.toLocaleString()} ms`
                  : ""}
              </dd>
            </div>
          )}
        </>
      )}
    </>
  );
}

function formatSyncTime(value: string | null): string {
  if (!value) return "Never";
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : date.toLocaleString();
}

function SemanticDiagnosticRows({
  semantic,
  indexing,
}: {
  semantic: SemanticDiagnostics;
  indexing: boolean;
}) {
  return (
    <>
      <div className="source-diagnostic-divider">
        <dt>Semantic index</dt>
        <dd>{indexing ? "indexing" : semantic.status}</dd>
      </div>
      <div>
        <dt>Embedding model</dt>
        <dd>
          {semantic.modelInstalled
            ? semantic.modelDisplayName
            : "not installed"}
        </dd>
      </div>
      <div>
        <dt>Paper coverage</dt>
        <dd>
          {semantic.indexedPapers} / {semantic.eligiblePapers}
          {semantic.stalePapers > 0 ? ` / ${semantic.stalePapers} stale` : ""}
        </dd>
      </div>
      <div>
        <dt>Passage coverage</dt>
        <dd>
          {semantic.indexedChunks.toLocaleString()} chunks /{" "}
          {semantic.chunkedDocuments} of {semantic.eligibleDocuments} documents
        </dd>
      </div>
      <div>
        <dt>Chunk profile</dt>
        <dd title={semantic.activeChunkingProfile.boundaryPolicy}>
          {semantic.activeChunkingProfile.id}:{" "}
          {semantic.activeChunkingProfile.targetTokens}/
          {semantic.activeChunkingProfile.overlapTokens}/
          {semantic.activeChunkingProfile.maximumTokens} tokens, cap{" "}
          {semantic.activeChunkingProfile.maximumChunksPerDocument}
        </dd>
      </div>
      <div>
        <dt>Long-document coverage</dt>
        <dd>
          max {semantic.maximumObservedChunksPerDocument} chunks /{" "}
          {semantic.documentsAtChunkLimit} at cap
        </dd>
      </div>
      <div>
        <dt>Document revisions</dt>
        <dd>
          {semantic.currentDocumentRevisionsIndexed} current /{" "}
          {semantic.staleDocumentRevisionsIndexed} stale
        </dd>
      </div>
      <div>
        <dt>Paper vectors</dt>
        <dd>
          {formatVectorConsistency(
            semantic.registeredPaperVectors,
            semantic.physicalPaperVectors,
          )}
        </dd>
      </div>
      <div>
        <dt>Passage vectors</dt>
        <dd>
          {formatVectorConsistency(
            semantic.registeredChunkVectors,
            semantic.physicalChunkVectors,
          )}
        </dd>
      </div>
      <div>
        <dt>Scoped retrieval</dt>
        <dd>
          {semantic.activeRetrievalProfile.scopedPath}, full indexed candidate
          set
        </dd>
      </div>
      <div>
        <dt>Context profile</dt>
        <dd>
          {semantic.activeRetrievalProfile.maximumEvidenceSources} sources /{" "}
          {semantic.activeRetrievalProfile.tokenAccounting}
        </dd>
      </div>
      <div>
        <dt>Local storage</dt>
        <dd>
          {formatBytes(
            semantic.modelStorageBytes +
              semantic.vectorIndexBytes +
              semantic.chunkMetadataBytes,
          )}
        </dd>
      </div>
    </>
  );
}

function formatVectorConsistency(
  registered: number,
  physical: number | null,
): string {
  if (physical === null) {
    return `${registered.toLocaleString()} registered / physical unavailable`;
  }
  const status = registered === physical ? "consistent" : "mismatch";
  return `${registered.toLocaleString()} registered / ${physical.toLocaleString()} physical (${status})`;
}

function formatBytes(value: number): string {
  if (value <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const exponent = Math.min(
    Math.floor(Math.log(value) / Math.log(1024)),
    units.length - 1,
  );
  return `${(value / 1024 ** exponent).toFixed(exponent === 0 ? 0 : 1)} ${units[exponent]}`;
}
