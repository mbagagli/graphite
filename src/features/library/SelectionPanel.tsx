import { Search, Undo2, X } from "lucide-react";
import { useEffect, useRef } from "react";

import type { ResearchItem } from "../../domain/research";
import "./selection.css";

interface SelectionPanelProps {
  papers: ResearchItem[];
  notice: string | null;
  canUndo: boolean;
  canSave: boolean;
  onClose: () => void;
  onSearch: () => void;
  onOpenPaper: (id: string) => void;
  onRemove: (id: string) => void;
  onClear: () => void;
  onUndo: () => void;
  onSave: () => void;
}

export function SelectionPanel({
  papers,
  notice,
  canUndo,
  canSave,
  onClose,
  onSearch,
  onOpenPaper,
  onRemove,
  onClear,
  onUndo,
  onSave,
}: SelectionPanelProps) {
  const panel = useRef<HTMLElement>(null);
  useEffect(() => {
    const frame = requestAnimationFrame(() => panel.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, []);

  return (
    <aside
      id="paper-selection"
      className="selection-panel"
      aria-labelledby="paper-selection-title"
      ref={panel}
      tabIndex={-1}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.stopPropagation();
          onClose();
        }
      }}
    >
      <header>
        <h2 id="paper-selection-title">
          Selected papers <span>{papers.length}</span>
        </h2>
        <button type="button" onClick={onClose} aria-label="Close selection">
          <X size={16} />
        </button>
      </header>
      <p className="selection-description">
        Collect across searches. Save a Lens to keep this set after restart.
      </p>
      <div className="selection-edit-tools">
        <button type="button" disabled={!papers.length} onClick={onClear}>
          Clear selection
        </button>
        <button type="button" disabled={!canUndo} onClick={onUndo}>
          <Undo2 size={14} /> Undo
        </button>
      </div>
      {notice && (
        <p className="selection-notice" role="status">
          {notice}
        </p>
      )}
      {papers.length ? (
        <ul className="selection-papers" aria-label="Selected papers">
          {papers.map((paper) => (
            <li key={paper.id}>
              <button
                type="button"
                className="selection-paper-title"
                onClick={() => onOpenPaper(paper.id)}
              >
                <strong>{paper.title}</strong>
                <small>
                  {paper.authors[0]?.name ?? "Unknown creator"} /{" "}
                  {paper.year ?? "No year"}
                </small>
              </button>
              <button
                type="button"
                className="selection-remove"
                aria-label={`Remove ${paper.title} from selection`}
                onClick={(event) => {
                  const row = event.currentTarget.closest("li");
                  const next =
                    row?.nextElementSibling ?? row?.previousElementSibling;
                  const target =
                    next?.querySelector<HTMLButtonElement>(".selection-remove");
                  (target ?? panel.current)?.focus();
                  onRemove(paper.id);
                }}
              >
                <X size={15} />
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <div className="selection-empty">
          <strong>Start with a paper that interests you</strong>
          <p>
            Check papers in search results, or use Add to selection in a paper's
            details.
          </p>
        </div>
      )}
      <footer>
        <button type="button" onClick={onSearch}>
          <Search size={14} /> Find papers
        </button>
        <button
          type="button"
          className="selection-save"
          disabled={!papers.length || !canSave}
          onClick={onSave}
        >
          Save as Lens
        </button>
        {!canSave && <small>Save Lenses in Graphite Desktop.</small>}
      </footer>
    </aside>
  );
}
