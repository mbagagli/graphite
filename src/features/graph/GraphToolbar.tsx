import { ChevronDown } from "lucide-react";
import { useId, type MouseEvent } from "react";
import type {
  GraphLayerVisibility,
  GraphRelationType,
  ResearchGraph,
} from "../../domain/graph";
import type { ResearchCollection } from "../../domain/research";
import type { NeighborhoodMode } from "../../services/contracts";

const connectionGroups: {
  label: string;
  types: [GraphRelationType, string][];
}[] = [
  {
    label: "Citations",
    types: [
      ["citation", "Citation links"],
      ["bibliographic-coupling", "Shared references"],
      ["co-cited", "Co-cited"],
    ],
  },
  {
    label: "Content similarity",
    types: [["semantically-similar", "Similar papers"]],
  },
  {
    label: "Library organization",
    types: [
      ["shared-creator", "Shared authors"],
      ["shared-tag", "Shared tags"],
      ["shared-collection", "Shared collections"],
      ["zotero-related", "Zotero related"],
    ],
  },
];
const citationActions: [NeighborhoodMode, string, string][] = [
  ["references", "References", "Works cited by any selected paper."],
  ["cited-by", "Cited by", "Local papers citing any selected paper."],
  [
    "shared-references",
    "Shared references",
    "Papers citing some of the same works as any selected paper.",
  ],
  [
    "co-cited",
    "Co-cited",
    "Works cited together with any selected paper by the local corpus.",
  ],
];
export interface GraphExplorationControls {
  selectedCount: number;
  collections: ResearchCollection[];
  similarScope: string;
  onScopeChange: (scope: string) => void;
  semanticAvailable: boolean;
  citationIndexed: boolean;
  pending: string | null;
  error: string | null;
  notice: string | null;
  onRetry: () => void;
  canRetry?: boolean;
  onSimilar: () => void;
  onCitations: (mode: NeighborhoodMode) => void;
  canGoBack: boolean;
  onBack: () => void;
  selectionConnectionsOnly: boolean;
  onSelectionConnectionsChange: (only: boolean) => void;
}
export function GraphToolbar({
  graph,
  layers,
  onToggleLayer,
  controls,
}: {
  graph: ResearchGraph;
  layers: GraphLayerVisibility;
  onToggleLayer: (type: GraphRelationType) => void;
  controls: GraphExplorationControls;
}) {
  const id = useId();
  const positionPopover = (
    event: MouseEvent<HTMLButtonElement>,
    suffix: string,
  ) => {
    const popup = document.getElementById(`${id}-${suffix}`);
    if (!popup) return;
    const rect = event.currentTarget.getBoundingClientRect();
    popup.style.left = `${Math.max(8, Math.min(rect.left, window.innerWidth - 350))}px`;
    popup.style.top = `${Math.max(8, Math.min(rect.bottom + 6, window.innerHeight - 450))}px`;
  };
  const disabled = !controls.selectedCount || Boolean(controls.pending);
  const enabledCount = graph.edges.filter((e) => layers[e.relation]).length;
  return (
    <div className="graph-exploration">
      <div className="graph-exploration-bar" aria-label="Graph exploration">
        <button
          type="button"
          disabled={disabled || !controls.semanticAvailable}
          title={
            !controls.selectedCount
              ? "Select a paper first"
              : !controls.semanticAvailable
                ? "Index papers in Data Source to find similar content"
                : "Find papers using the selection's combined title and abstract representation"
          }
          onClick={controls.onSimilar}
        >
          Find similar
        </button>
        <button
          type="button"
          className="graph-scope-trigger"
          aria-label="Similarity search scope"
          popoverTarget={`${id}-similarity-scope`}
          onClick={(event) => positionPopover(event, "similarity-scope")}
          disabled={Boolean(controls.pending)}
        >
          <span>
            {controls.collections.find(
              (collection) =>
                `collection:${collection.id}` === controls.similarScope,
            )?.name ?? "Whole library"}
          </span>
          <ChevronDown size={13} aria-hidden="true" />
        </button>
        <div
          id={`${id}-similarity-scope`}
          popover="auto"
          className="graph-control-popover"
          aria-label="Similarity search scope"
        >
          <fieldset disabled={Boolean(controls.pending)}>
            <legend>Find similar within</legend>
            {[
              { value: "library", label: "Whole library" },
              ...[...controls.collections]
                .sort((a, b) => a.name.localeCompare(b.name))
                .map((collection) => ({
                  value: `collection:${collection.id}`,
                  label: collection.name,
                })),
            ].map(({ value, label }) => (
              <label key={value}>
                <input
                  type="radio"
                  name={`${id}-similarity-scope`}
                  value={value}
                  checked={controls.similarScope === value}
                  onChange={() => controls.onScopeChange(value)}
                />
                {label}
              </label>
            ))}
          </fieldset>
        </div>
        <button
          type="button"
          popoverTarget={`${id}-citations`}
          onClick={(event) => positionPopover(event, "citations")}
          disabled={disabled}
          title={
            !controls.selectedCount
              ? "Select a paper first"
              : "Explore connections to any selected paper"
          }
        >
          Explore citations
        </button>
        <div
          id={`${id}-citations`}
          popover="auto"
          className="graph-control-popover"
          aria-label="Explore citations"
        >
          <strong>Explore citations</strong>
          {citationActions.map(([mode, label, description]) => (
            <button
              type="button"
              key={mode}
              popoverTarget={`${id}-citations`}
              popoverTargetAction="hide"
              onClick={() => controls.onCitations(mode)}
            >
              <span>{label}</span>
              <small>{description}</small>
            </button>
          ))}
        </div>
        <button
          type="button"
          popoverTarget={`${id}-connections`}
          onClick={(event) => positionPopover(event, "connections")}
        >
          Show connections
        </button>
        <div
          id={`${id}-connections`}
          popover="auto"
          className="graph-control-popover"
          aria-label="Show connections"
        >
          <fieldset>
            <legend>Connection visibility</legend>
            {[false, true].map((only) => (
              <label key={String(only)}>
                <input
                  type="radio"
                  name={`${id}-visibility`}
                  checked={controls.selectionConnectionsOnly === only}
                  onChange={() => controls.onSelectionConnectionsChange(only)}
                />
                {only
                  ? "Selection connections only"
                  : "All enabled connections"}
              </label>
            ))}
          </fieldset>
          {connectionGroups.map((group) => (
            <fieldset key={group.label}>
              <legend>{group.label}</legend>
              {group.types.map(([type, label]) => (
                <label key={type}>
                  <input
                    type="checkbox"
                    checked={layers[type]}
                    onChange={() => onToggleLayer(type)}
                  />
                  {label}
                  {!graph.availableRelations.includes(type) && (
                    <small>No links in this view</small>
                  )}
                </label>
              ))}
            </fieldset>
          ))}
          <small>Visibility only. This does not rearrange papers.</small>
        </div>
      </div>
      {controls.selectedCount > 1 && (
        <small className="graph-exploration-note">
          Similar uses the combined representation; citations include
          connections to any selected paper.
        </small>
      )}
      {controls.pending && <p role="status">{controls.pending}</p>}
      {controls.error && (
        <p role="alert">
          {controls.error}{" "}
          {controls.canRetry && (
            <button type="button" onClick={controls.onRetry}>
              Retry
            </button>
          )}
        </p>
      )}
      {controls.notice && <p role="status">{controls.notice}</p>}
      {controls.selectionConnectionsOnly && !controls.selectedCount && (
        <p role="status">Select a paper to see its enabled connections.</p>
      )}
      {!enabledCount && !controls.pending && (
        <p className="graph-exploration-note">
          {!controls.citationIndexed
            ? "Citations have not been indexed. Open Data Source to index them, or select papers to Find similar."
            : graph.edges.length
              ? "No enabled connections in this view. Show connections controls visibility; citation coverage may be incomplete."
              : "No connections in this view. Citation coverage can be incomplete; select papers to explore."}
        </p>
      )}
    </div>
  );
}
