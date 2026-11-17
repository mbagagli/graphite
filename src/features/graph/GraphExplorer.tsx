import { CornerUpLeft, Focus, Minus, Plus, RotateCcw, X } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import {
  Fragment,
  lazy,
  Suspense,
  useRef,
  useState,
  type RefObject,
} from "react";
import { IconButton } from "../../components/IconButton";
import { useTheme } from "../../design-system/theme";
import type {
  GraphLayerVisibility,
  GraphNode,
  ResearchGraph,
} from "../../domain/graph";
import type {
  ResearchItem,
  ResearchLens,
  ResearchSelection,
  Topic,
} from "../../domain/research";
import { GraphNavigationHelp, GraphSelection } from "./GraphOverlays";
import { GraphToolbar, type GraphExplorationControls } from "./GraphToolbar";
import { GraphBoundary } from "./GraphBoundary";
import {
  type GraphMode,
  type GraphGrouping,
  type GraphSession,
} from "./graphLayout";
import { TopicControls } from "./TopicControls";
import type { GraphControls, GraphFocusRequest } from "./GraphCanvas";
const GraphCanvas = lazy(() =>
  import("./GraphCanvas").then((module) => ({ default: module.GraphCanvas })),
);

interface GraphExplorerProps {
  onPrepareTopics: () => void;
  active?: boolean;
  activePaperId: string | null;
  mode: GraphMode;
  grouping: GraphGrouping;
  onGroupingChange: (grouping: GraphGrouping) => void;
  onModeChange: (mode: GraphMode) => void;
  session: RefObject<GraphSession>;
  controls: RefObject<GraphControls | null>;
  restoreSerial: number;
  exploration: GraphExplorationControls;
  focusRequest?: GraphFocusRequest | null;
  graph: ResearchGraph;
  papers: ResearchItem[];
  topics: Topic[];
  selection: ResearchSelection;
  selectedEntityId: string | null;
  layers: GraphLayerVisibility;
  highlightedPaperIds: string[];
  activeLens: ResearchLens | null;
  scopeLabel: string;
  onSelectNode: (node: GraphNode, additive: boolean) => void;
  onSelectEdge: (edgeId: string) => void;
  onClearSelection: () => void;
  onToggleLayer: (relation: keyof GraphLayerVisibility) => void;
  onClearLens: () => void;
}

export function GraphExplorer({
  active = true,
  onPrepareTopics,
  activePaperId,
  mode,
  grouping,
  onGroupingChange,
  onModeChange,
  session,
  controls,
  restoreSerial,
  exploration,
  focusRequest,
  graph,
  papers,
  topics,
  selection,
  selectedEntityId,
  layers,
  highlightedPaperIds,
  activeLens,
  scopeLabel,
  onSelectNode,
  onSelectEdge,
  onClearSelection,
  onToggleLayer,
  onClearLens,
}: GraphExplorerProps) {
  const {
    state: { theme },
  } = useTheme();
  const [hoveredNode, setHoveredNode] = useState<GraphNode | null>(null);
  const previewNode =
    hoveredNode && graph.nodes.includes(hoveredNode) ? hoveredNode : null;
  const previewId = previewNode ? previewNode.paperId : activePaperId;
  const [graphFailure, setGraphFailure] = useState(false);
  const [retry, setRetry] = useState(0);
  const focusSerialRef = useRef(0);
  const changeMode = (next: GraphMode) => onModeChange(next);
  const focusPaper = (id: string) => controls.current?.focus(id);
  const zoomIn = () => controls.current?.zoom(0.8);
  const zoomOut = () => controls.current?.zoom(1.25);
  const resetCamera = () => controls.current?.fit();

  return (
    <section className="graph-explorer" aria-label="Research graph explorer">
      {active && !graphFailure && (
        <GraphBoundary key={retry} onFailure={() => setGraphFailure(true)}>
          <Suspense
            fallback={<div className="view-loading">Preparing network...</div>}
          >
            <GraphCanvas
              graph={graph}
              activePaperId={activePaperId}
              selectionConnectionsOnly={exploration.selectionConnectionsOnly}
              restoreSerial={restoreSerial}
              mode={mode}
              grouping={grouping}
              topics={topics}
              theme={theme}
              selectedIds={
                selectedEntityId
                  ? [...selection.paperIds, selectedEntityId]
                  : selection.paperIds
              }
              highlightedIds={highlightedPaperIds}
              lensIds={activeLens?.paperIds}
              layers={layers}
              controlsRef={controls}
              sessionRef={session}
              focusRequest={focusRequest}
              focusSerialRef={focusSerialRef}
              onSelectNode={onSelectNode}
              onHoverNode={setHoveredNode}
              onSelectEdge={onSelectEdge}
              onClearSelection={onClearSelection}
              onFailure={() => setGraphFailure(true)}
            />
          </Suspense>
        </GraphBoundary>
      )}
      {graphFailure && (
        <div className="graph-failure" role="status">
          <strong>The graph display was interrupted</strong>
          <button
            type="button"
            onClick={() => {
              setRetry((value) => value + 1);
              setGraphFailure(false);
            }}
          >
            Retry graph
          </button>
        </div>
      )}

      <div className="graph-title-block">
        <span className="eyebrow">RESEARCH GRAPH</span>
        <div className="graph-view-heading">
          <button
            type="button"
            className="graph-back"
            disabled={!exploration.canGoBack && !exploration.pending}
            onClick={exploration.onBack}
            aria-label="Back to previous graph"
          >
            <CornerUpLeft size={14} />
            Back
          </button>
          <h1 title={scopeLabel}>{scopeLabel}</h1>
        </div>
        <p>
          {graph.nodes.length.toLocaleString()} visible works /{" "}
          {graph.totalNodeCount.toLocaleString()} available
        </p>
      </div>

      <GraphToolbar
        graph={graph}
        layers={layers}
        onToggleLayer={onToggleLayer}
        controls={exploration}
      />

      {graph.nodes.length === 0 && (
        <div className="graph-empty-state">
          <strong>
            {activeLens
              ? activeLens.paperIds.length === 0
                ? "This Lens is empty"
                : "No Lens papers are currently available"
              : "No research items cached"}
          </strong>
          <span>
            {activeLens
              ? activeLens.paperIds.length === 0
                ? "Choose All papers, select papers, then Edit this Lens to add them."
                : "Saved membership is retained. Restore the source papers or edit this Lens."
              : "Open Data Source and sync a running Zotero library."}
          </span>
        </div>
      )}

      <AnimatePresence>
        {activeLens && (
          <motion.div
            className="active-lens-pill"
            initial={{ opacity: 0, y: -8, scale: 0.96 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -6, scale: 0.97 }}
            transition={{ duration: 0.28 }}
          >
            <Focus size={14} />
            <div>
              <span>ACTIVE LENS</span>
              <strong>{activeLens.name}</strong>
            </div>
            <button
              type="button"
              onClick={onClearLens}
              aria-label="Clear active Lens"
            >
              <X size={14} />
            </button>
          </motion.div>
        )}
      </AnimatePresence>

      <div className="graph-tools">
        <div
          className="graph-mode-switch"
          role="group"
          aria-label="Graph dimensions"
        >
          {(["3d", "2d"] as const).map((value) => (
            <Fragment key={value}>
              <button
                type="button"
                aria-pressed={mode === value}
                onClick={() => changeMode(value)}
                title={value === "3d" ? "Expand into 3D" : "Collapse into 2D"}
              >
                {value.toUpperCase()}
              </button>
              {value === "2d" && <GraphNavigationHelp mode={mode} />}
            </Fragment>
          ))}
        </div>
        <IconButton
          label="Focus selection"
          icon={Focus}
          size="small"
          onClick={() => {
            const id = selectedEntityId ?? activePaperId;
            if (id) focusPaper(id);
          }}
        />
        <IconButton label="Zoom in" icon={Plus} size="small" onClick={zoomIn} />
        <IconButton
          label="Zoom out"
          icon={Minus}
          size="small"
          onClick={zoomOut}
        />
        <span />
        <IconButton
          label="Fit graph"
          icon={RotateCcw}
          size="small"
          onClick={resetCamera}
        />
      </div>

      {graph.nodes.length > 0 && (
        <TopicControls
          graph={graph}
          topics={topics}
          grouping={grouping}
          onGroupingChange={onGroupingChange}
          onPrepare={onPrepareTopics}
        />
      )}

      {(previewNode || selection.paperIds.length > 0) && (
        <GraphSelection
          paper={papers.find((paper) => paper.id === previewId)}
          title={
            previewNode?.label ??
            graph.nodes.find((node) => node.paperId === previewId)?.label
          }
          additionalCount={previewNode ? 0 : selection.paperIds.length - 1}
        />
      )}
    </section>
  );
}
