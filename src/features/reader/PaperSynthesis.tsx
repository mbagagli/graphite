import { LoaderCircle } from "lucide-react";

import { assistantDisplayName } from "../../shared/productIdentity";
import type { usePaperSynthesis } from "./usePaperSynthesis";

interface PaperSynthesisProps {
  documentId: string;
  synthesis: ReturnType<typeof usePaperSynthesis>;
  onOpenAgent: () => void;
  onNavigatePage: (pageNumber: number) => void;
}

export function PaperSynthesis({
  documentId,
  synthesis,
  onOpenAgent,
  onNavigatePage,
}: PaperSynthesisProps) {
  return (
    <section className="paper-synthesis" aria-live="polite">
      <h3>{assistantDisplayName} says</h3>
      {synthesis.environmentLoading ? (
        <div className="paper-synthesis-status">
          <LoaderCircle className="spin" size={13} /> Checking local setup...
        </div>
      ) : !synthesis.available ? (
        <>
          <p>
            {synthesis.environmentError ??
              `Set up ${assistantDisplayName} to synthesize this paper from indexed evidence.`}
          </p>
          <button
            type="button"
            className="paper-synthesis-action"
            onClick={onOpenAgent}
          >
            Open {assistantDisplayName}
          </button>
        </>
      ) : synthesis.draft ? (
        <>
          <SynthesisText
            value={synthesis.draft}
            result={synthesis.result}
            documentId={documentId}
            onNavigatePage={onNavigatePage}
          />
          {synthesis.running ? (
            <div className="paper-synthesis-status">
              <LoaderCircle className="spin" size={13} />
              {synthesis.stageMessage}
            </div>
          ) : (
            <div className="paper-synthesis-provenance">
              <span>
                {synthesis.result?.sources.length ?? 0} local sources /{" "}
                {synthesis.result?.modelLabel ?? "grounded agent"}
              </span>
              <button type="button" onClick={() => void synthesis.generate()}>
                Regenerate
              </button>
            </div>
          )}
        </>
      ) : synthesis.error ? (
        <>
          <p>{synthesis.error}</p>
          <div className="paper-synthesis-actions">
            <button type="button" onClick={() => void synthesis.generate()}>
              Retry
            </button>
            <button type="button" onClick={onOpenAgent}>
              Open {assistantDisplayName}
            </button>
          </div>
        </>
      ) : (
        <>
          <p>
            Generate a concise synthesis from the indexed passages available to
            Graphite's grounded local agent.
          </p>
          <button
            type="button"
            className="paper-synthesis-action"
            onClick={() => void synthesis.generate()}
            disabled={!documentId || synthesis.running}
          >
            Synthesize paper
          </button>
        </>
      )}
    </section>
  );
}

function SynthesisText({
  value,
  result,
  documentId,
  onNavigatePage,
}: {
  value: string;
  result: ReturnType<typeof usePaperSynthesis>["result"];
  documentId: string;
  onNavigatePage: (pageNumber: number) => void;
}) {
  return (
    <div className="paper-synthesis-answer">
      {value.split("\n").map((line, lineIndex) => (
        <p key={`${lineIndex}-${line}`}>
          {line.split(/(\[S\d+\])/g).map((part, partIndex) => {
            const sourceId = part.match(/^\[(S\d+)\]$/)?.[1];
            const source = result?.sources.find(
              (candidate) => candidate.sourceId === sourceId,
            );
            return sourceId &&
              source?.documentId === documentId &&
              source.pageStart ? (
              <button
                type="button"
                key={`${lineIndex}-${partIndex}`}
                onClick={() => onNavigatePage(source.pageStart ?? 1)}
                title={`Open source on page ${source.pageStart}`}
              >
                {part}
              </button>
            ) : (
              <span key={`${lineIndex}-${partIndex}`}>{part}</span>
            );
          })}
        </p>
      ))}
    </div>
  );
}
