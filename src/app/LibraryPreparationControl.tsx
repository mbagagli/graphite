import { Download, FolderOpen, Play, Square } from "lucide-react";
import { useState } from "react";
import { demoManifest } from "../adapters/demo/demoData";
import type { LibraryPreparationController } from "./useLibraryPreparation";

export function LibraryPreparationControl({
  preparation,
  demo,
  disabled,
  desktop,
  embeddingInstalled = false,
  libraryAvailable = true,
}: {
  preparation: LibraryPreparationController;
  demo: boolean;
  disabled: boolean;
  desktop: boolean;
  embeddingInstalled?: boolean;
  libraryAvailable?: boolean;
}) {
  const [choosing, setChoosing] = useState(false);
  const { progress, busy, archive, archiveError, demoCached } = preparation;
  if (!desktop)
    return (
      <p className="source-privacy-note">
        Explore the real metadata here. Open Graphite Desktop to prepare PDFs,
        local document search and source-grounded answers.
      </p>
    );
  const needsArchive = demo && !demoCached;
  const archiveBytes =
    archive?.bytes ??
    demoManifest.papers.reduce((sum, paper) => sum + paper.pdfBytes, 0);
  const offerArchive = demo && (needsArchive || progress?.status === "error");
  const resumable =
    progress && ["cancelled", "needs-model"].includes(progress.status);
  const retry = progress && ["error", "partial"].includes(progress.status);
  const start = () => {
    if (offerArchive && !choosing) {
      setChoosing(true);
      return;
    }
    void preparation.start();
  };
  const label = busy
    ? "Preparing library"
    : retry
      ? "Retry preparation"
      : resumable
        ? "Resume preparation"
        : choosing && needsArchive
          ? "Download and prepare"
          : "Prepare library for AI";
  const total = progress?.total;
  const determinate = total !== null && total !== undefined && total > 0;
  const count =
    progress?.unit === "bytes"
      ? `${Math.round(progress.completed / 1_000_000)} / ${Math.round((total ?? 0) / 1_000_000)} MB`
      : `${progress?.completed ?? 0} / ${total ?? 0} ${progress?.unit ?? "documents"}`;

  return (
    <section
      className="library-preparation"
      aria-label="Prepare library for AI"
    >
      <p>
        {demo
          ? `Prepare the ${demoManifest.papers.length} real demo papers for document search and TYRION.`
          : "Prepare available papers in this Zotero library for document search and TYRION."}
        {needsArchive &&
          ` Optional PDF download: approximately ${Math.round(archiveBytes / 1_000_000)} MB.`}
      </p>
      {!libraryAvailable && (
        <p>Connect Zotero and select a library to begin.</p>
      )}
      <div className="library-preparation-actions">
        <button
          type="button"
          className="library-prepare-button"
          disabled={
            busy ||
            disabled ||
            !libraryAvailable ||
            (choosing && needsArchive && !archive?.url)
          }
          onClick={start}
        >
          <Play size={14} aria-hidden="true" /> {label}
        </button>
        {busy && (
          <button type="button" onClick={() => void preparation.cancel()}>
            <Square size={12} aria-hidden="true" /> Cancel
          </button>
        )}
      </div>
      {progress && (
        <div className="library-preparation-progress">
          {busy && (
            <progress
              aria-label={progress.detail}
              max={determinate ? total : undefined}
              value={
                determinate ? Math.min(progress.completed, total) : undefined
              }
            />
          )}
          <div
            role={progress.status === "error" ? "alert" : "status"}
            aria-live="polite"
            aria-atomic="true"
          >
            {progress.status === "needs-model" && (
              <strong>
                {embeddingInstalled
                  ? "Ready to resume"
                  : "Embedding model needed"}
              </strong>
            )}
            <span>
              {progress.status === "needs-model" && embeddingInstalled
                ? "Embedding model installed. Resume preparation to finish document indexing."
                : progress.detail}
            </span>
            {determinate && <small>{count}</small>}
          </div>
        </div>
      )}
      {offerArchive && choosing && !busy && (
        <div className="library-archive-choice">
          {archive?.url ? (
            <p>
              <Download size={13} aria-hidden="true" />
              The verified Zenodo archive is downloaded automatically. No models
              are downloaded.
            </p>
          ) : (
            <p role="status">
              The Zenodo download is not published yet. You can prepare the demo
              using its local ZIP.
            </p>
          )}
          <button
            type="button"
            disabled={disabled}
            onClick={() => void preparation.start(true)}
          >
            <FolderOpen size={14} aria-hidden="true" /> Already downloaded?
            Choose ZIP
          </button>
          {archiveError && <p role="alert">{archiveError}</p>}
        </div>
      )}
      {progress && progress.issues.length > 0 && (
        <details className="library-preparation-issues">
          <summary>
            {progress.issues.length}{" "}
            {progress.issues.length === 1 ? "item needs" : "items need"}{" "}
            attention
          </summary>
          <ul>
            {progress.issues.map((issue, index) => (
              <li key={`${issue.itemId}-${index}`}>
                <strong>{issue.title}</strong>
                <span>{issue.message}</span>
              </li>
            ))}
          </ul>
        </details>
      )}
      <p className="library-preparation-note">
        Models are installed separately below and in TYRION Setup. Completed
        work stays on this computer.
      </p>
    </section>
  );
}
