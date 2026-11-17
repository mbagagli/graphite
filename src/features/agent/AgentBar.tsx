import {
  AlertTriangle,
  ArrowUp,
  BookOpen,
  Bot,
  Check,
  ChevronDown,
  Clock3,
  Cpu,
  Database,
  Download,
  ExternalLink,
  FileDown,
  FileStack,
  Focus,
  History,
  Gpu,
  MemoryStick,
  ListTree,
  LoaderCircle,
  Network,
  RotateCcw,
  Settings2,
  ShieldCheck,
  Square,
  Trash2,
  X,
} from "lucide-react";
import { isDesktopRuntime } from "../../services/runtime";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect, useId, useRef, useState } from "react";

import type {
  ResearchItem,
  ResearchLens,
  ResearchSelection,
} from "../../domain/research";
import type {
  AgentEnvironment,
  AgentEvidence,
  AgentRunResult,
  AgentSettings,
  ConversationSummary,
  SemanticDiagnostics,
} from "../../services/contracts";
import { assistantDisplayName } from "../../shared/productIdentity";
import {
  type ReaderAgentContext,
  isManagedModelUnavailable,
  useAgentController,
} from "./useAgentController";

import { InferenceControls } from "./InferenceControls";
import { agentPreparation } from "./agentPreparation";

const dockSpring = {
  type: "spring",
  stiffness: 460,
  damping: 24,
  mass: 0.8,
} as const;

type AgentPanelTab = "answer" | "sources" | "history" | "setup";

interface AgentBarProps {
  semantic: SemanticDiagnostics;
  availableTextDocuments: number;
  sourceMode: "demo" | "zotero";
  onLibrarySetup: () => void;
  selection: ResearchSelection;
  activePaper: ResearchItem | null;
  activeLens: ResearchLens | null;
  activeCollectionId: string | null;
  view: "graph" | "analytics" | "reader";
  readerContext: ReaderAgentContext | null;
  prompt: string | null;
  lensAskRequest?: number;
  onPromptConsumed: () => void;
  onRunComplete: (result: AgentRunResult) => void;
  onOpenSource: (source: AgentEvidence, openInReader: boolean) => void;
  onCreateTemporaryLens: (query: string, itemIds: string[]) => void;
}

export function AgentBar({
  semantic,
  availableTextDocuments,
  sourceMode,
  onLibrarySetup,
  selection,
  activePaper,
  activeLens,
  activeCollectionId,
  view,
  readerContext,
  prompt,
  lensAskRequest = 0,
  onPromptConsumed,
  onRunComplete,
  onOpenSource,
  onCreateTemporaryLens,
}: AgentBarProps) {
  const contextId = useId();
  const contextRef = useRef<HTMLDivElement>(null);
  const reducedMotion = useReducedMotion();
  const dockTransition = reducedMotion
    ? { duration: 0.1 }
    : { ...dockSpring, opacity: { duration: 0.12 } };
  const [dismissedPreparation, setDismissedPreparation] = useState<
    string | null
  >(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const launcherRef = useRef<HTMLButtonElement>(null);
  const restoreLauncherFocus = useRef(false);
  const [composerOpen, setComposerOpen] = useState(false);
  const [panelOpen, setPanelOpen] = useState(false);
  const [tab, setTab] = useState<AgentPanelTab>("answer");
  const [expandedSource, setExpandedSource] = useState<string | null>(null);
  const {
    environment,
    settings,
    setSettings,
    apiKey,
    setApiKey,
    query,
    setQuery,
    contextChips,
    selectedOnly,
    lensOnly,
    setLensOnly,
    setSelectedOnly,
    removedContext,
    removeContext,
    restoreContext,
    conversations,
    installingProfile,
    installMessage,
    providerCheck,
    externalReady,
    running,
    stage,
    stageMessage,
    draft,
    result,
    error,
    conversationId,
    submit,
    installModel,
    persistSettings,
    settingsStatus,
    settingsError,
    checkExternal,
    openConversation,
    newConversation,
    exportConversation,
    deleteConversation,
    createLens,
    cancel,
    unloadModel,
  } = useAgentController({
    selection,
    activePaper,
    activeLens,
    activeCollectionId,
    view,
    readerContext,
    onRunComplete,
    onCreateTemporaryLens,
  });

  useEffect(() => {
    if (!lensAskRequest) return;
    const handle = window.setTimeout(() => {
      setLensOnly(true);
      setSelectedOnly(false);
      restoreContext();
    }, 0);
    return () => window.clearTimeout(handle);
  }, [lensAskRequest, setLensOnly, setSelectedOnly, restoreContext]);

  useEffect(() => {
    if (!prompt) return;
    const handle = window.setTimeout(() => {
      setComposerOpen(true);
      setQuery(prompt);
      setPanelOpen(true);
      setTab("answer");
      onPromptConsumed();
    }, 0);
    return () => window.clearTimeout(handle);
  }, [onPromptConsumed, prompt, setQuery]);

  const preparation = !isDesktopRuntime
    ? {
        title: "Local research models run in the desktop app",
        message:
          "This browser preview explores real paper metadata. Open Graphite Desktop to download the PDFs, prepare their text, and ask source-grounded questions.",
        action: "library" as const,
      }
    : agentPreparation(
        semantic,
        isManagedModelUnavailable(environment, settings),
        sourceMode === "demo",
        availableTextDocuments,
        settings?.providerKind === "openai-compatible" && !externalReady,
      );

  const preparationKey = !settings
    ? (error ?? "loading")
    : (preparation?.title ?? null);

  const removableContextChips = contextChips.filter(
    (chip) =>
      chip.kind !== "library" && (!selectedOnly || chip.kind === "selection"),
  );

  useEffect(() => {
    if (composerOpen) {
      inputRef.current?.focus();
    } else if (restoreLauncherFocus.current) {
      launcherRef.current?.focus();
      restoreLauncherFocus.current = false;
    }
  }, [composerOpen]);

  const closeAgent = () => {
    restoreLauncherFocus.current = true;
    setComposerOpen(false);
    setPanelOpen(false);
    contextRef.current?.hidePopover();
  };

  return (
    <div className={`agent-region ${panelOpen ? "is-open" : ""}`}>
      <AnimatePresence>
        {panelOpen && (
          <motion.section
            className="agent-workspace"
            aria-label={`${assistantDisplayName} evidence workspace`}
            initial={{ opacity: 0, y: reducedMotion ? 0 : 18 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: reducedMotion ? 0 : 12 }}
            transition={dockTransition}
          >
            <header className="agent-workspace-header">
              <div>
                <span className="eyebrow">
                  {assistantDisplayName.toUpperCase()} / LOCAL RESEARCH AGENT
                </span>
                <strong>
                  {tab === "setup"
                    ? "Local setup"
                    : (result?.intent.replaceAll("-", " ") ??
                      "Evidence workspace")}
                </strong>
              </div>
              <div className="agent-workspace-status">
                <span className={running ? "is-active" : ""} />
                {running
                  ? stage.replaceAll("-", " ")
                  : result
                    ? "grounded answer"
                    : stage === "cancelled"
                      ? "cancelled"
                      : !settings
                        ? "setup unavailable"
                        : preparation
                          ? "preparation needed"
                          : "Ready to ask TYRION"}
              </div>
              <button
                type="button"
                onClick={closeAgent}
                aria-label={`Close ${assistantDisplayName}`}
              >
                <X size={15} />
              </button>
            </header>

            <nav
              className="agent-tabs"
              aria-label={`${assistantDisplayName} workspace sections`}
            >
              {(
                [
                  ["answer", Bot, "Answer"],
                  [
                    "sources",
                    FileStack,
                    `Sources ${result?.sources.length ?? 0}`,
                  ],
                  ["history", History, "History"],
                  ["setup", Settings2, "Local setup"],
                ] as const
              ).map(([id, Icon, label]) => (
                <button
                  type="button"
                  key={id}
                  className={tab === id ? "is-active" : ""}
                  onClick={() => setTab(id)}
                  aria-pressed={tab === id}
                >
                  <Icon size={13} />
                  {label}
                </button>
              ))}
            </nav>

            <div className="agent-workspace-body">
              {tab === "answer" && (
                <AnswerPanel
                  draft={draft}
                  result={result}
                  running={running}
                  stage={stage}
                  stageMessage={stageMessage}
                  error={error}
                  onCitation={(sourceId) => {
                    setExpandedSource(sourceId);
                    setTab("sources");
                  }}
                  onCreateLens={createLens}
                  onHighlight={() => result && onRunComplete(result)}
                  onExport={() => void exportConversation()}
                />
              )}
              {tab === "sources" && (
                <SourcesPanel
                  sources={result?.sources ?? []}
                  expandedSource={expandedSource}
                  onExpand={setExpandedSource}
                  onOpenSource={(source, openInReader) => {
                    onOpenSource(source, openInReader);
                    closeAgent();
                  }}
                />
              )}
              {tab === "history" && (
                <HistoryPanel
                  conversations={conversations}
                  activeId={conversationId}
                  onOpen={(id) =>
                    void openConversation(id, () => setTab("answer"))
                  }
                  onDelete={(id) => {
                    if (
                      !window.confirm(
                        `Delete this ${assistantDisplayName} conversation and its run records?`,
                      )
                    ) {
                      return;
                    }
                    void deleteConversation(id);
                  }}
                  onNew={() => {
                    newConversation();
                    setTab("answer");
                  }}
                />
              )}
              {tab === "setup" && environment && settings && (
                <SetupPanel
                  environment={environment}
                  settings={settings}
                  apiKey={apiKey}
                  installingProfile={installingProfile}
                  installMessage={installMessage}
                  providerCheck={providerCheck}
                  error={error}
                  onSettings={(next) => void persistSettings(next)}
                  onDraftSettings={setSettings}
                  settingsStatus={settingsStatus}
                  settingsError={settingsError}
                  onApiKey={setApiKey}
                  onInstall={(id) => void installModel(id)}
                  onSave={() => void persistSettings()}
                  onCheck={() => void checkExternal()}
                  onUnload={() => void unloadModel()}
                />
              )}
            </div>
          </motion.section>
        )}
      </AnimatePresence>

      <AnimatePresence initial={false}>
        {composerOpen ? (
          <motion.div
            key="composer"
            className="agent-composer-container"
            initial={{
              opacity: 0,
              scale: reducedMotion ? 1 : 0.88,
              y: reducedMotion ? 0 : 12,
            }}
            animate={{ opacity: 1, scale: 1, y: 0, pointerEvents: "auto" }}
            exit={{
              opacity: 0,
              scale: reducedMotion ? 1 : 0.88,
              y: reducedMotion ? 0 : 12,
              pointerEvents: "none",
            }}
            transition={dockTransition}
          >
            {composerOpen &&
              preparationKey &&
              preparationKey !== dismissedPreparation && (
                <div className="agent-preparation" role="status">
                  <button
                    type="button"
                    className="agent-preparation-close"
                    aria-label="Dismiss preparation notice"
                    onClick={() => {
                      setDismissedPreparation(preparationKey);
                      inputRef.current?.focus();
                    }}
                  >
                    <X size={14} />
                  </button>
                  <div>
                    <strong>
                      {!settings
                        ? error
                          ? "Tyrion setup could not load"
                          : "Loading Tyrion setup"
                        : preparation?.title}
                    </strong>
                    <span>
                      {!settings
                        ? (error ??
                          "Checking the local answer model. Questions will be available when setup has loaded.")
                        : preparation?.message}
                    </span>
                  </div>
                  {settings && preparation && (
                    <button
                      type="button"
                      className="agent-preparation-action"
                      onClick={() => {
                        setDismissedPreparation(preparationKey);
                        if (preparation.action === "library") {
                          setPanelOpen(false);
                          onLibrarySetup();
                        } else {
                          setPanelOpen(true);
                          setTab("setup");
                        }
                      }}
                    >
                      {preparation.action === "library"
                        ? "Open library setup"
                        : "Open answer model setup"}
                    </button>
                  )}
                </div>
              )}

            {composerOpen && (
              <>
                <div
                  ref={contextRef}
                  id={contextId}
                  popover="auto"
                  className="agent-context-popover"
                  aria-label={`${assistantDisplayName} search context`}
                >
                  <fieldset className="agent-scope-choice" disabled={running}>
                    <legend>Answer scope</legend>
                    {[false, true].map((only) => (
                      <label key={String(only)}>
                        <input
                          type="radio"
                          name={`${contextId}-scope`}
                          value={only ? "selected-only" : "broader"}
                          checked={!lensOnly && selectedOnly === only}
                          onChange={() => {
                            setSelectedOnly(only);
                            setLensOnly(false);
                          }}
                        />
                        {only ? "Selected papers only" : "Broader search"}
                      </label>
                    ))}
                    <label>
                      <input
                        type="radio"
                        name={`${contextId}-scope`}
                        checked={lensOnly}
                        disabled={!activeLens}
                        onChange={() => {
                          setLensOnly(true);
                          setSelectedOnly(false);
                        }}
                      />
                      This Lens only
                    </label>
                  </fieldset>
                  <div
                    className="agent-context-chips"
                    aria-label={`${assistantDisplayName} context`}
                  >
                    {lensOnly && (
                      <span className="agent-scope-hint" role="status">
                        This Lens only; earlier answers excluded. Up to 64
                        available papers.
                      </span>
                    )}
                    {selectedOnly && (
                      <span className="agent-scope-hint" role="status">
                        {selection.paperIds.length === 0 ||
                        removedContext.includes("selection")
                          ? "Select papers to ask a question."
                          : `${selection.paperIds.length} selected; earlier answers excluded.`}
                      </span>
                    )}
                    {removableContextChips.map((chip) => (
                      <button
                        type="button"
                        key={chip.kind}
                        onClick={() => removeContext(chip.kind)}
                        title="Remove from this request"
                      >
                        {chip.kind === "lens" ? (
                          <Focus size={11} />
                        ) : (
                          <Network size={11} />
                        )}
                        {chip.label}
                        <X size={10} />
                      </button>
                    ))}
                    {removedContext.length > 0 && (
                      <button type="button" onClick={restoreContext}>
                        <RotateCcw size={10} />
                        Restore context
                      </button>
                    )}
                  </div>
                </div>
                <div className="agent-bar">
                  <div className="agent-controls">
                    <button
                      type="button"
                      className="agent-identity"
                      onClick={closeAgent}
                      aria-label={`Collapse ${assistantDisplayName}`}
                      aria-expanded={true}
                      title={`Collapse ${assistantDisplayName}`}
                    >
                      <span className="agent-icon">
                        <Bot size={17} />
                      </span>
                    </button>
                    <button
                      type="button"
                      className="agent-setup-trigger"
                      aria-expanded={panelOpen && tab === "setup"}
                      onClick={() => {
                        setPanelOpen(true);
                        setTab("setup");
                      }}
                    >
                      Setup
                    </button>
                    <button
                      type="button"
                      className="agent-context-trigger"
                      popoverTarget={contextId}
                      title={
                        lensOnly
                          ? "This Lens only"
                          : selectedOnly
                            ? "Selected papers only"
                            : "Broader search"
                      }
                      onClick={(event) => {
                        const popup = contextRef.current;
                        if (!popup) return;
                        const rect =
                          event.currentTarget.getBoundingClientRect();
                        popup.style.left = `${Math.max(
                          8,
                          Math.min(rect.left, window.innerWidth - 348),
                        )}px`;
                        popup.style.bottom = `${window.innerHeight - rect.top + 8}px`;
                      }}
                    >
                      {lensOnly ? "This Lens only" : "Search Context"}
                      <ChevronDown size={12} aria-hidden="true" />
                    </button>
                  </div>
                  <form
                    onSubmit={(event) => {
                      event.preventDefault();
                      void submit((target) => {
                        setPanelOpen(true);
                        setTab(target);
                      });
                    }}
                  >
                    <input
                      ref={inputRef}
                      autoFocus
                      name="research-agent-query"
                      autoComplete="off"
                      value={query}
                      onChange={(event) => setQuery(event.target.value)}
                      placeholder={`Ask ${assistantDisplayName} with local evidence`}
                      aria-label={`Ask ${assistantDisplayName}`}
                    />
                    {running ? (
                      <button
                        type="button"
                        className="agent-stop"
                        onClick={() => void cancel()}
                        aria-label="Stop generation"
                      >
                        <Square size={13} fill="currentColor" />
                      </button>
                    ) : (
                      <button
                        type="submit"
                        disabled={!query.trim() || !settings}
                        aria-label="Submit question"
                      >
                        <ArrowUp size={16} />
                      </button>
                    )}
                  </form>
                  <button
                    type="button"
                    className="agent-close"
                    aria-label={`Close ${assistantDisplayName}`}
                    onClick={closeAgent}
                  >
                    <X size={15} />
                  </button>
                </div>
              </>
            )}
          </motion.div>
        ) : (
          <motion.button
            key="launcher"
            type="button"
            ref={launcherRef}
            className="agent-launcher"
            aria-label={`Open ${assistantDisplayName}`}
            aria-expanded={false}
            initial={{ opacity: 0, scale: reducedMotion ? 1 : 0.88 }}
            animate={{ opacity: 1, scale: 1, pointerEvents: "auto" }}
            exit={{
              opacity: 0,
              scale: reducedMotion ? 1 : 0.88,
              pointerEvents: "none",
            }}
            transition={dockTransition}
            onClick={() => {
              setComposerOpen(true);
              if (result || draft || running || error) setPanelOpen(true);
            }}
          >
            <Bot size={17} /> {assistantDisplayName.toUpperCase()}
            {running && <LoaderCircle size={14} className="is-spinning" />}
          </motion.button>
        )}
      </AnimatePresence>
    </div>
  );
}

function AnswerPanel({
  draft,
  result,
  running,
  stage,
  stageMessage,
  error,
  onCitation,
  onCreateLens,
  onHighlight,
  onExport,
}: {
  draft: string;
  result: AgentRunResult | null;
  running: boolean;
  stage: string;
  stageMessage: string;
  error: string | null;
  onCitation: (sourceId: string) => void;
  onCreateLens: () => void;
  onHighlight: () => void;
  onExport: () => void;
}) {
  return (
    <div className="agent-answer-panel">
      {running && (
        <div className="agent-stage-line">
          <LoaderCircle className="is-spinning" size={13} />
          <span>{stageMessage}</span>
        </div>
      )}
      {!running && stage === "cancelled" && (
        <div className="agent-stage-line is-cancelled">
          <Square size={11} fill="currentColor" />
          <span>{stageMessage}</span>
        </div>
      )}
      {error && (
        <div className="agent-error-line">
          <AlertTriangle size={13} />
          {error}
        </div>
      )}
      {!draft && !error && !running && (
        <div className="agent-empty-state">
          <Bot size={23} />
          <strong>Ask a scoped research question</strong>
          <span>
            {assistantDisplayName} resolves context, runs bounded read-only
            retrieval, and exposes every source used.
          </span>
        </div>
      )}
      {draft && (
        <div className="agent-answer-text" aria-live="polite">
          <SafeGroundedText value={draft} onCitation={onCitation} />
          {running && <span className="agent-stream-caret" />}
        </div>
      )}
      {result && (
        <>
          <div className="agent-run-strip">
            <span>
              <ShieldCheck size={12} /> {result.sources.length} sources
            </span>
            <span>
              <Clock3 size={12} /> {formatDuration(result.timings.totalMs)}
            </span>
            <span>{result.contextTokens.toLocaleString()} context tokens</span>
            <span>{result.modelLabel}</span>
          </div>
          {result.limitations.length > 0 && (
            <div className="agent-limitations">
              <AlertTriangle size={12} />
              <div>
                {result.limitations.map((limitation) => (
                  <span key={limitation}>{limitation}</span>
                ))}
              </div>
            </div>
          )}
          <div className="agent-actions">
            <button type="button" onClick={onHighlight}>
              <Focus size={12} /> Highlight sources
            </button>
            <button type="button" onClick={onCreateLens}>
              <Network size={12} /> Temporary Lens
            </button>
            <button type="button" onClick={onExport}>
              <FileDown size={12} /> Export Markdown
            </button>
          </div>
        </>
      )}
    </div>
  );
}

function SafeGroundedText({
  value,
  onCitation,
}: {
  value: string;
  onCitation: (sourceId: string) => void;
}) {
  return (
    <>
      {value.split("\n").map((line, lineIndex) => {
        const content = line.split(/(\[S\d+\])/g).map((part, partIndex) => {
          const sourceId = part.match(/^\[(S\d+)\]$/)?.[1];
          return sourceId ? (
            <button
              type="button"
              className="agent-inline-citation"
              key={`${lineIndex}-${partIndex}`}
              onClick={() => onCitation(sourceId)}
            >
              {part}
            </button>
          ) : (
            <span key={`${lineIndex}-${partIndex}`}>{part}</span>
          );
        });
        if (!line.trim()) return <br key={`line-${lineIndex}`} />;
        if (line.startsWith("### "))
          return <h4 key={`line-${lineIndex}`}>{content}</h4>;
        if (line.startsWith("## "))
          return <h3 key={`line-${lineIndex}`}>{content}</h3>;
        if (line.startsWith("# "))
          return <h2 key={`line-${lineIndex}`}>{content}</h2>;
        return line.startsWith("|") ? (
          <div className="agent-table-line" key={`line-${lineIndex}`}>
            {content}
          </div>
        ) : (
          <p key={`line-${lineIndex}`}>{content}</p>
        );
      })}
    </>
  );
}

function SourcesPanel({
  sources,
  expandedSource,
  onExpand,
  onOpenSource,
}: {
  sources: AgentEvidence[];
  expandedSource: string | null;
  onExpand: (sourceId: string | null) => void;
  onOpenSource: (source: AgentEvidence, openInReader: boolean) => void;
}) {
  if (sources.length === 0)
    return <div className="agent-empty-state">No source set is active.</div>;
  return (
    <div className="agent-source-list">
      {sources.map((source) => {
        const expanded = source.sourceId === expandedSource;
        return (
          <article
            key={source.sourceId}
            className={`${expanded ? "is-expanded" : ""} ${source.stale ? "is-stale" : ""}`}
          >
            <button
              type="button"
              className="agent-source-heading"
              onClick={() => onExpand(expanded ? null : source.sourceId)}
            >
              <span>{source.sourceId}</span>
              <div>
                <strong>{source.itemTitle}</strong>
                <small>
                  {source.kind.replaceAll("-", " ")} ·{" "}
                  {source.pageStart
                    ? `p. ${source.pageStart}`
                    : "page unavailable"}
                </small>
              </div>
              {source.eligibleForScientificClaim ? (
                <ShieldCheck size={13} />
              ) : (
                <Database size={13} />
              )}
            </button>
            {expanded && (
              <div className="agent-source-detail">
                {source.stale && (
                  <div className="agent-source-warning">
                    <AlertTriangle size={12} /> The indexed source changed after
                    this answer.
                  </div>
                )}
                <p>
                  {source.excerpt ??
                    "Metadata evidence only; no passage was supplied."}
                </p>
                <dl>
                  <div>
                    <dt>Provider</dt>
                    <dd>{source.provider}</dd>
                  </div>
                  <div>
                    <dt>Retrieval</dt>
                    <dd>
                      {source.retrievalModes.join(" + ") || "deterministic"}
                    </dd>
                  </div>
                  <div>
                    <dt>Revision</dt>
                    <dd>{source.documentRevisionId ?? "metadata"}</dd>
                  </div>
                </dl>
                <div className="agent-source-actions">
                  <button
                    type="button"
                    onClick={() => onOpenSource(source, false)}
                  >
                    <Network size={12} /> Show in Graph
                  </button>
                  <button
                    type="button"
                    disabled={!source.documentId}
                    onClick={() => onOpenSource(source, true)}
                  >
                    <BookOpen size={12} /> Open in Reader
                  </button>
                </div>
              </div>
            )}
          </article>
        );
      })}
    </div>
  );
}

function HistoryPanel({
  conversations,
  activeId,
  onOpen,
  onDelete,
  onNew,
}: {
  conversations: ConversationSummary[];
  activeId: string | undefined;
  onOpen: (id: string) => void;
  onDelete: (id: string) => void;
  onNew: () => void;
}) {
  return (
    <div className="agent-history-panel">
      <button type="button" className="agent-new-thread" onClick={onNew}>
        <Bot size={13} /> New local conversation
      </button>
      {conversations.length === 0 ? (
        <div className="agent-empty-state">No saved conversations yet.</div>
      ) : (
        conversations.map((conversation) => (
          <div
            className={`agent-history-row ${activeId === conversation.id ? "is-active" : ""}`}
            key={conversation.id}
          >
            <button type="button" onClick={() => onOpen(conversation.id)}>
              <strong>{conversation.title}</strong>
              <span>
                {conversation.turnCount} turns ·{" "}
                {formatTimestamp(conversation.updatedAt)}
              </span>
            </button>
            <button
              type="button"
              aria-label={`Delete ${conversation.title}`}
              onClick={() => onDelete(conversation.id)}
            >
              <Trash2 size={13} />
            </button>
          </div>
        ))
      )}
    </div>
  );
}

export function SetupPanel({
  environment,
  settings,
  apiKey,
  installingProfile,
  installMessage,
  providerCheck,
  error,
  onSettings,
  onDraftSettings,
  settingsStatus,
  settingsError,
  onApiKey,
  onInstall,
  onSave,
  onCheck,
  onUnload,
}: {
  environment: AgentEnvironment;
  settings: AgentSettings;
  apiKey: string;
  installingProfile: string | null;
  installMessage: string | null;
  providerCheck: string | null;
  error: string | null;
  onSettings: (settings: AgentSettings) => void;
  onDraftSettings: (settings: AgentSettings) => void;
  settingsStatus: string | null;
  settingsError: string | null;
  onApiKey: (value: string) => void;
  onInstall: (profileId: string) => void;
  onSave: () => void;
  onCheck: () => void;
  onUnload: () => void;
}) {
  const memory = (environment.hardware.totalMemoryBytes / 1024 ** 3).toFixed(1);
  return (
    <div className="agent-setup-panel">
      <div className="agent-setup-heading">
        <strong>Answer model</strong>
        <span>
          {settings.providerKind === "graphite-managed"
            ? environment.modelProfiles.find(
                (profile) => profile.id === settings.modelProfile,
              )?.installed
              ? "Selected model ready"
              : "Selected model needs a download"
            : "Model run by another app on this computer"}
        </span>
      </div>
      {settings.providerKind === "graphite-managed" && (
        <div className="agent-model-profiles">
          {environment.modelProfiles.map((profile) => (
            <article
              key={profile.id}
              className={
                settings.modelProfile === profile.id ? "is-selected" : ""
              }
            >
              <button
                type="button"
                className="agent-profile-choice"
                aria-pressed={settings.modelProfile === profile.id}
                onClick={() =>
                  onSettings({ ...settings, modelProfile: profile.id })
                }
              >
                <span className="agent-profile-radio">
                  {settings.modelProfile === profile.id && <Check size={10} />}
                </span>
                <div>
                  <strong>{profile.label}</strong>
                  <small>
                    Up to {profile.contextCapacity / 1024}k context ·{" "}
                    {formatBytes(profile.expectedBytes)} · {profile.license}
                  </small>
                </div>
                <span
                  className={`agent-profile-state ${profile.installed ? "is-ready" : ""}`}
                >
                  {profile.installed ? "READY" : "NOT INSTALLED"}
                </span>
              </button>
              {!profile.installed && (
                <button
                  type="button"
                  className="agent-install-button"
                  disabled={Boolean(installingProfile)}
                  onClick={() => onInstall(profile.id)}
                >
                  {installingProfile === profile.id ? (
                    <LoaderCircle className="is-spinning" size={13} />
                  ) : (
                    <Download size={13} />
                  )}
                  Download and verify {formatBytes(profile.expectedBytes)}
                </button>
              )}
            </article>
          ))}
          {installMessage && (
            <div className="agent-install-progress">{installMessage}</div>
          )}
        </div>
      )}
      {settings.providerKind === "graphite-managed" ? (
        <InferenceControls
          settings={settings}
          hardware={environment.hardware}
          capacity={
            environment.modelProfiles.find(
              (profile) => profile.id === settings.modelProfile,
            )?.contextCapacity ?? 8192
          }
          onSettings={onSettings}
        />
      ) : (
        <p className="agent-setup-explanation">
          Answers run in your other local AI app. Choose CPU/GPU and its context
          window there. Graphite still handles search, evidence, and citations.
          The connection is configured below.
        </p>
      )}
      <div className="agent-detail-setting">
        <span>Answer detail</span>
        <button
          type="button"
          aria-pressed={settings.answerDetail === "concise"}
          className={settings.answerDetail === "concise" ? "is-active" : ""}
          onClick={() => onSettings({ ...settings, answerDetail: "concise" })}
        >
          Concise
        </button>
        <button
          type="button"
          aria-pressed={settings.answerDetail === "detailed"}
          className={settings.answerDetail === "detailed" ? "is-active" : ""}
          onClick={() => onSettings({ ...settings, answerDetail: "detailed" })}
        >
          Detailed
        </button>
      </div>
      <div className="agent-settings-status" role="status">
        {settingsStatus}
      </div>
      {settingsError && (
        <div className="agent-error-line" role="alert">
          <span>{settingsError}</span>
          <button type="button" onClick={onSave}>
            Retry save
          </button>
        </div>
      )}
      {providerCheck && (
        <div className="agent-provider-check">
          <Check size={12} /> {providerCheck}
        </div>
      )}
      {error && (
        <div className="agent-error-line">
          <AlertTriangle size={12} /> {error}
        </div>
      )}
      <details className="agent-hardware-details">
        <summary>Hardware and memory</summary>
        <div className="agent-hardware-line">
          <Cpu size={14} aria-hidden="true" />
          <span>
            {environment.hardware.logicalCpus} logical CPUs
            {environment.hardware.cpuName &&
              ` · ${environment.hardware.cpuName}`}
          </span>
        </div>
        <div className="agent-hardware-line">
          <MemoryStick size={14} aria-hidden="true" />
          <span>{memory} GiB RAM</span>
        </div>
        {environment.hardware.graphicsAdapters?.length ? (
          <div className="agent-detected-graphics">
            <strong>Installed graphics cards</strong>
            {environment.hardware.graphicsAdapters.map((name, index) => (
              <div className="agent-hardware-line" key={`${name}-${index}`}>
                <Gpu size={14} aria-hidden="true" />
                <span>{name}</span>
              </div>
            ))}
            <p>Hardware detection is separate from AI acceleration support.</p>
          </div>
        ) : null}
        {Boolean(environment.hardware.gpuDevices?.length) && (
          <strong>Available to Graphite's inference engine</strong>
        )}
        {environment.hardware.gpuDevices?.map((device) => (
          <div
            className="agent-hardware-line"
            key={`${device.backend}-${device.index}`}
          >
            <Gpu size={14} aria-hidden="true" />
            <span>
              {device.name} (
              {device.kind === "integrated" ? "integrated GPU" : "GPU"})
              {device.totalMemoryBytes > 0
                ? ` · ${formatBytes(device.availableMemoryBytes)} free / ${formatBytes(device.totalMemoryBytes)} total`
                : " · memory limit not reported"}
            </span>
          </div>
        ))}
        <div
          className="agent-hardware-status"
          role="status"
          aria-label="Managed inference hardware"
        >
          <span>
            Graphite engine status: {environment.hardware.accelerationBackend}
          </span>
          <strong>{environment.hardware.recommendedProfile} suggested</strong>
          <span>{environment.hardware.note}</span>
        </div>
        <button
          type="button"
          className="agent-unload-button"
          onClick={onUnload}
        >
          Release Graphite model memory
        </button>
        <p>
          Downloaded models stay on disk. This unloads Graphite's built-in
          answer model from RAM or GPU memory; it does not stop another AI app.
        </p>
      </details>
      <details className="agent-advanced-settings">
        <summary>Advanced: answer model</summary>
        <p>
          Graphite finds relevant passages in your papers. The answer model uses
          those passages to write a response. Choose which app runs that model:
        </p>
        <div
          className="agent-provider-toggle"
          role="group"
          aria-label="App that runs the answer model"
        >
          <button
            type="button"
            className={
              settings.providerKind === "graphite-managed" ? "is-active" : ""
            }
            aria-pressed={settings.providerKind === "graphite-managed"}
            onClick={() =>
              onSettings({ ...settings, providerKind: "graphite-managed" })
            }
          >
            <Database size={13} aria-hidden="true" /> Built-in model
          </button>
          <button
            type="button"
            className={
              settings.providerKind === "openai-compatible" ? "is-active" : ""
            }
            aria-pressed={settings.providerKind === "openai-compatible"}
            onClick={() =>
              onSettings({ ...settings, providerKind: "openai-compatible" })
            }
          >
            <ExternalLink size={13} aria-hidden="true" /> Another app on this
            computer
          </button>
        </div>
        <p>
          <strong>Built-in model (recommended):</strong> Graphite downloads and
          runs the model for you. No other AI app is needed.
        </p>
        <p>
          <strong>Another app on this computer:</strong> Reuse a model you
          already run in Ollama, LM Studio, or another compatible app. That app
          must be running. Graphite still handles paper search and citations.
        </p>
        <p>
          Graphite connects only to this computer. Direct connections to LAN
          servers, a VPS, or the GPT and Claude cloud APIs are not supported.
        </p>
        {settings.providerKind === "openai-compatible" && (
          <div className="agent-external-settings">
            <p>
              Enable the other app's OpenAI-compatible local server, then copy
              its address and model name below. "OpenAI-compatible" describes
              the connection format; it does not connect Graphite to OpenAI.
            </p>
            <label>
              Server address on this computer
              <input
                name="loopback-endpoint"
                onBlur={onSave}
                type="url"
                autoComplete="off"
                spellCheck={false}
                value={settings.externalEndpoint ?? ""}
                placeholder="http://127.0.0.1:11434"
                onChange={(event) =>
                  onDraftSettings({
                    ...settings,
                    externalEndpoint: event.target.value,
                  })
                }
              />
            </label>
            <label>
              Model name in the other app
              <input
                name="model-identifier"
                onBlur={onSave}
                autoComplete="off"
                spellCheck={false}
                value={settings.externalModel ?? ""}
                placeholder="qwen3:8b"
                onChange={(event) =>
                  onDraftSettings({
                    ...settings,
                    externalModel: event.target.value,
                  })
                }
              />
            </label>
            <label>
              Context configured in the other app (tokens)
              <input
                name="context-tokens"
                onBlur={onSave}
                type="number"
                inputMode="numeric"
                min={2048}
                max={131072}
                value={settings.externalContextTokens ?? 8192}
                onChange={(event) =>
                  onDraftSettings({
                    ...settings,
                    externalContextTokens: Number(event.target.value),
                  })
                }
              />
            </label>
            <label>
              Local server API key (optional)
              <input
                name="session-credential"
                type="password"
                autoComplete="off"
                spellCheck={false}
                value={apiKey}
                placeholder="Never persisted"
                aria-describedby="agent-local-server-key-help"
                onChange={(event) => onApiKey(event.target.value)}
              />
            </label>
            <p id="agent-local-server-key-help">
              Leave blank unless your local server requires a key. A GPT or
              Claude API key does not enable cloud access here. This key is kept
              only for the current session.
            </p>
            <div className="agent-external-actions">
              <button type="button" onClick={onCheck}>
                Test local connection
              </button>
            </div>
            <p>
              Set the model's GPU and context limit in the other app; these
              fields do not change its settings. Graphite sends it your
              question, selected passages and relevant conversation history.
              Check that app's privacy settings for any onward connections.
            </p>
          </div>
        )}
      </details>
      <details className="agent-runtime-details">
        <summary>
          <ListTree size={12} /> Runtime and policy details
        </summary>
        <span>
          {environment.runtimeName} · {environment.runtimeVersion}
        </span>
        <span>
          Retrieval step limit: {environment.toolBudget} · maximum evidence
          budget: {environment.contextBudgetTokens.toLocaleString()} tokens
        </span>
        <span>{environment.rerankerStatus}</span>
        <div className="agent-trust-statement">
          <ShieldCheck size={16} />
          <div>
            <strong>Local and explicit by default</strong>
            <span>{environment.trustStatement}</span>
          </div>
        </div>

        <span>
          Linked inference backend:{" "}
          {environment.hardware.compiledBackend ??
            environment.hardware.accelerationBackend}
        </span>
      </details>
    </div>
  );
}

function formatBytes(bytes: number): string {
  return `${(bytes / 1024 ** 3).toFixed(2)} GiB`;
}

function formatDuration(milliseconds: number): string {
  return milliseconds < 1000
    ? `${milliseconds} ms`
    : `${(milliseconds / 1000).toFixed(1)} s`;
}

function formatTimestamp(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : date.toLocaleString();
}
