import { ChevronUp, Network, Tags, X } from "lucide-react";
import { useId } from "react";
import type { ResearchGraph } from "../../domain/graph";
import type { Topic } from "../../domain/research";
import type { GraphGrouping } from "./graphLayout";

export function TopicControls({
  graph,
  topics,
  grouping,
  onGroupingChange,
  onPrepare,
}: {
  onPrepare: () => void;
  graph: ResearchGraph;
  topics: Topic[];
  grouping: GraphGrouping;
  onGroupingChange: (grouping: GraphGrouping) => void;
}) {
  const id = useId();
  const ready = topics.some((topic) => topic.origin === "semantic-cluster");
  const counts = new Map<string, number>();
  for (const node of graph.nodes) {
    const topic = node.topicId ?? "";
    counts.set(topic, (counts.get(topic) ?? 0) + 1);
  }
  const coverage = topics.find((topic) => topic.analyzed != null);
  const noClearGroups = topics[0]?.name === "No clear semantic groups";
  const topicMap = new Map(topics.map((topic) => [topic.id, topic]));
  const visible = [...counts].sort(([a], [b]) =>
    (topicMap.get(a)?.name ?? "No topic").localeCompare(
      topicMap.get(b)?.name ?? "No topic",
    ),
  );
  return (
    <div className="graph-topic-controls">
      <button
        type="button"
        popoverTarget={id}
        onClick={(event) => {
          const popup = document.getElementById(id);
          if (!popup) return;
          const rect = event.currentTarget.getBoundingClientRect();
          popup.style.left = `${rect.left}px`;
          popup.style.bottom = `${window.innerHeight - rect.top + 8}px`;
        }}
      >
        <Tags size={13} aria-hidden="true" />
        TOPIC MAP
        <ChevronUp size={12} aria-hidden="true" />
      </button>
      <div id={id} popover="auto" className="topic-map-popover">
        <header>
          <strong>Visible topics</strong>
          <button
            type="button"
            popoverTarget={id}
            popoverTargetAction="hide"
            aria-label="Close topic map"
          >
            <X size={14} />
          </button>
        </header>
        <p>
          {ready
            ? "Automatic similarity groups from indexed paper content. Neutral papers are unassigned."
            : noClearGroups
              ? "No clear semantic groups were found. Citations remain available."
              : "Prepare your library to discover topics. Semantic indexing is required; citations remain available."}
        </p>
        {topics[0]?.name.startsWith("Topic analysis failed") && (
          <p role="status">{topics[0].name}</p>
        )}
        {coverage && (
          <p>
            {coverage.analyzed} / {coverage.eligible} papers analyzed.
          </p>
        )}
        {!ready && (
          <button
            className="topic-prepare-action"
            type="button"
            onClick={onPrepare}
          >
            Prepare library for AI
          </button>
        )}
        {ready && (
          <ul>
            {visible.map(([topicId, count]) => {
              const topic = topicMap.get(topicId);
              return (
                <li key={topicId}>
                  <i
                    style={{
                      background: topic?.color ?? "var(--color-text-muted)",
                    }}
                  />
                  <span title={topic?.representatives?.join("\n")}>
                    {topic?.name ?? "No topic"}
                  </span>
                  <small>{count}</small>
                </li>
              );
            })}
          </ul>
        )}
      </div>
      <button
        type="button"
        aria-pressed={grouping === "topics"}
        disabled={!ready || counts.size < 2}
        title={
          !ready
            ? "Prepare the semantic index to discover topics"
            : counts.size < 2
              ? "This view contains only one topic group"
              : "Arrange works by discovered topic; click again to restore the relationship layout"
        }
        onClick={() =>
          onGroupingChange(grouping === "topics" ? "relations" : "topics")
        }
      >
        <Network size={13} aria-hidden="true" />
        Group By Topic
      </button>
    </div>
  );
}
