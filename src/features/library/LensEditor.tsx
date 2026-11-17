import { useRef, useState } from "react";
import type { ResearchItem, ResearchLens } from "../../domain/research";
import {
  lensMembershipChanges,
  MAX_LENS_PAPERS,
} from "../../domain/researchLens";
import { LensPaperPicker } from "./LensPaperPicker";
import { ModalDialog } from "../../components/ModalDialog";
import { services } from "../../services/runtime";
import "./lenses.css";

interface LensEditorProps {
  libraryId: string;
  lens: ResearchLens | null;
  selection: string[];
  papers: ResearchItem[];
  onClose: () => void;
  onSaved: (lens: ResearchLens) => Promise<void>;
  onDeleted: (id: string) => Promise<void>;
}

export function LensEditor({
  libraryId,
  lens,
  selection,
  papers,
  onClose,
  onSaved,
  onDeleted,
}: LensEditorProps) {
  const [name, setName] = useState(lens?.name ?? "");
  const [question, setQuestion] = useState(lens?.description ?? "");
  const [ids, setIds] = useState(lens?.paperIds ?? selection);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [addedTitles, setAddedTitles] = useState<Record<string, string>>({});
  const addPapersButton = useRef<HTMLButtonElement>(null);
  const deleteButton = useRef<HTMLButtonElement>(null);
  const editorBody = useRef<HTMLFormElement>(null);
  const titles = new Map([
    ...papers.map((paper) => [paper.id, paper.title] as const),
    ...Object.entries(addedTitles),
  ]);
  const missing = new Set(lens?.unavailablePaperIds ?? []);
  const saved = lens?.saved && lens.revision !== undefined;
  const additions = selection.filter((id) => !ids.includes(id));
  const originalIds = saved ? lens.paperIds : [];
  const changes = lensMembershipChanges(originalIds, ids);

  function finishAdding(): void {
    setSearchOpen(false);
    requestAnimationFrame(() => addPapersButton.current?.focus());
  }

  function keepLens(): void {
    setConfirmDelete(false);
    setError(null);
    requestAnimationFrame(() => deleteButton.current?.focus());
  }

  function addPaper(paper: ResearchItem): void {
    setAddedTitles((current) => ({ ...current, [paper.id]: paper.title }));
    setIds((current) =>
      current.includes(paper.id) || current.length >= MAX_LENS_PAPERS
        ? current
        : [...current, paper.id],
    );
  }

  async function save(): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      const result = await services.lenses.save({
        ...(saved ? { id: lens.id, revision: lens.revision } : {}),
        libraryId,
        name,
        description: question,
        paperIds: ids,
      });
      await onSaved(result);
      onClose();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function remove(): Promise<void> {
    if (busy || !lens || lens.revision === undefined) return;
    setBusy(true);
    setError(null);
    try {
      await services.lenses.delete(libraryId, lens.id, lens.revision);
      await onDeleted(lens.id);
      onClose();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <ModalDialog
      title={
        confirmDelete
          ? "Delete Lens"
          : searchOpen
            ? "Add papers"
            : saved
              ? "Edit Lens"
              : "Save Lens"
      }
      titleId="lens-editor-title"
      onClose={() => {
        if (busy) return;
        if (confirmDelete) keepLens();
        else if (searchOpen) finishAdding();
        else onClose();
      }}
    >
      <form
        ref={editorBody}
        className={`modal-dialog-body lens-editor ${searchOpen ? "is-searching" : ""}`}
        onSubmit={(event) => {
          event.preventDefault();
          if (!busy && !searchOpen && !confirmDelete) void save();
        }}
        aria-busy={busy}
      >
        {confirmDelete ? (
          <fieldset disabled={busy} className="lens-delete-confirm">
            <p>
              Delete "{lens?.name}"? Its papers, PDFs and Zotero records will be
              kept. This cannot be undone.
            </p>
            {error && (
              <p role="alert" className="lens-error">
                {error}
              </p>
            )}
            <div className="lens-editor-actions">
              <button
                type="button"
                onClick={keepLens}
                ref={(button) => button?.focus()}
              >
                Keep Lens
              </button>
              <button type="button" onClick={() => void remove()}>
                {busy ? "Deleting..." : "Confirm delete Lens"}
              </button>
            </div>
          </fieldset>
        ) : searchOpen ? (
          <LensPaperPicker
            name={name}
            query={searchQuery}
            onQueryChange={setSearchQuery}
            memberIds={ids}
            originalIds={originalIds}
            onAdd={addPaper}
            onDone={finishAdding}
          />
        ) : (
          <>
            <p>
              A fixed set of papers for one investigation. Saved in Graphite for
              this library; your Zotero collections stay unchanged.
            </p>
            <fieldset disabled={busy}>
              <label>
                Name
                <input
                  required
                  maxLength={120}
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                />
              </label>
              <label>
                Research question (optional)
                <textarea
                  maxLength={4000}
                  value={question}
                  onChange={(event) => setQuestion(event.target.value)}
                />
              </label>
              <div className="lens-editor-heading">
                <h3>
                  {ids.length} {ids.length === 1 ? "paper" : "papers"}
                </h3>
                <button
                  type="button"
                  ref={addPapersButton}
                  onClick={() => {
                    setSearchOpen(true);
                    editorBody.current?.scrollTo(0, 0);
                  }}
                >
                  Add papers
                </button>
              </div>
              <p className="lens-draft-changes" role="status">
                {changes.added.length} added / {changes.removed.length} removed
              </p>
              {selection.length > 0 && (
                <div className="lens-add-selection">
                  <button
                    type="button"
                    disabled={additions.length === 0}
                    onClick={() =>
                      setIds((current) => [
                        ...new Set([...current, ...selection]),
                      ])
                    }
                  >
                    Add selected papers ({additions.length})
                  </button>
                  {additions.length === 0 && (
                    <small>
                      Your selected papers are already in this Lens.
                    </small>
                  )}
                </div>
              )}
              {missing.size > 0 && (
                <p>Missing papers remain in the Lens until you remove them.</p>
              )}
              <ul className="lens-members">
                {ids.map((id) => (
                  <li key={id}>
                    <span>
                      {titles.get(id) ?? lens?.paperTitles?.[id] ?? id}
                      {changes.added.includes(id) && (
                        <small className="lens-member-added">
                          Added to draft
                        </small>
                      )}
                      {missing.has(id) && (
                        <small>Unavailable in this library</small>
                      )}
                    </span>
                    <button
                      type="button"
                      aria-label={`Remove ${titles.get(id) ?? lens?.paperTitles?.[id] ?? id}`}
                      onClick={() =>
                        setIds((current) =>
                          current.filter((member) => member !== id),
                        )
                      }
                    >
                      Remove
                    </button>
                  </li>
                ))}
              </ul>
              {ids.length === 0 && (
                <p>This Lens is empty. You can save it and add papers later.</p>
              )}
              {ids.length > MAX_LENS_PAPERS && (
                <p role="alert">
                  A Lens can contain at most {MAX_LENS_PAPERS} papers.
                </p>
              )}
              {error && (
                <p role="alert" className="lens-error">
                  {error}
                </p>
              )}
              <div className="lens-editor-actions">
                <button
                  type="submit"
                  disabled={!name.trim() || ids.length > MAX_LENS_PAPERS}
                >
                  {busy ? "Saving..." : "Save Lens"}
                </button>
                <button type="button" onClick={onClose}>
                  Cancel
                </button>
                {saved && (
                  <button
                    ref={deleteButton}
                    type="button"
                    onClick={() => {
                      setError(null);
                      setConfirmDelete(true);
                      editorBody.current?.scrollTo(0, 0);
                    }}
                  >
                    Delete Lens
                  </button>
                )}
              </div>
            </fieldset>
          </>
        )}
      </form>
    </ModalDialog>
  );
}
