import type { ResearchItem } from "../../domain/research";
import { ModalDialog } from "../../components/ModalDialog";

import "./analytics.css";

interface YearPapersDialogProps {
  year: number | null;
  papers: ResearchItem[];
  onClose: () => void;
  onSelectPaper: (paperId: string) => void;
}

export function YearPapersDialog({
  year,
  papers,
  onClose,
  onSelectPaper,
}: YearPapersDialogProps) {
  const label = year === null ? "No year" : String(year);
  return (
    <ModalDialog
      title={`${label} - ${papers.length} ${papers.length === 1 ? "paper" : "papers"}`}
      titleId="year-papers-title"
      className="year-papers-dialog"
      onClose={onClose}
    >
      <div className="modal-dialog-body">
        <p>
          Current loaded view. Sorted by title, then record identifier.
          {year === null && " These records have no valid publication year."}
        </p>
        {papers.length === 0 ? (
          <p role="status">No papers match this year in the current view.</p>
        ) : (
          <ol className="year-paper-list">
            {papers.map((paper) => (
              <li key={paper.id}>
                <button
                  type="button"
                  onClick={() => {
                    onClose();
                    onSelectPaper(paper.id);
                  }}
                >
                  <strong>{paper.title}</strong>
                  <span>
                    {paper.authors.map((author) => author.name).join(", ") ||
                      "Unknown creator"}
                  </span>
                  <small>
                    {paper.venue ?? "Venue unavailable"} / {label}
                    {paper.doi ? ` / DOI: ${paper.doi}` : ""}
                  </small>
                </button>
              </li>
            ))}
          </ol>
        )}
      </div>
    </ModalDialog>
  );
}
