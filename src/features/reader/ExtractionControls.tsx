import { useCallback, useEffect, useState } from "react";
import type { DoclingStatus } from "../../services/contracts";
import { services } from "../../services/runtime";

export function ExtractionControls({
  documentId,
  onChanged,
}: {
  documentId: string;
  onChanged: () => Promise<void>;
}) {
  const [status, setStatus] = useState<DoclingStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const refresh = useCallback(async () => {
    setStatus(await services.document.doclingStatus(documentId));
  }, [documentId]);
  useEffect(() => {
    let active = true;
    const poll = async () => {
      try {
        const next = await services.document.doclingStatus(documentId);
        if (active) setStatus(next);
      } catch (error) {
        if (active) setNotice(String(error));
      }
    };
    void poll();
    const interval = window.setInterval(() => void poll(), 1500);
    return () => {
      active = false;
      window.clearInterval(interval);
    };
  }, [documentId]);
  const running = busy || status?.job.status === "running";
  const perform = async (action: () => Promise<void>, message: string) => {
    setBusy(true);
    setNotice("");
    try {
      await action();
      await refresh();
      await onChanged();
      setNotice(message);
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="extraction-controls" aria-label="PDF extraction">
      <h3>Text for search and answers</h3>
      <p>{status?.message ?? "Checking local extraction options..."}</p>
      <button
        type="button"
        disabled={!status?.available || running}
        onClick={() =>
          void perform(
            () => services.document.extractWithDocling(documentId),
            "Docling text saved. Update this paper's index to use it in answers.",
          )
        }
      >
        {running ? "Processing..." : "Extract with Docling"}
      </button>
      {status?.job.documentId === documentId &&
        status.job.status === "running" && (
          <button
            type="button"
            onClick={() =>
              void services.document
                .cancelDocling(documentId)
                .catch((error) => setNotice(String(error)))
            }
          >
            Cancel extraction
          </button>
        )}
      {status && status.choices.length > 0 && (
        <>
          <label htmlFor="reader-extraction">Saved extraction</label>
          <select
            id="reader-extraction"
            disabled={running}
            value={status.choices.find((choice) => choice.active)?.id ?? ""}
            onChange={(event) =>
              void perform(
                () =>
                  services.document.selectExtraction(
                    documentId,
                    event.target.value,
                  ),
                "Saved extraction restored. Update this paper's index before asking questions.",
              )
            }
          >
            {!status.choices.some((choice) => choice.active) && (
              <option value="">Choose saved text</option>
            )}
            {status.choices.map((choice) => (
              <option key={choice.id} value={choice.id}>
                {choice.provider.startsWith("docling")
                  ? "Docling"
                  : "Original parser"}{" "}
                - {new Date(choice.createdAt).toLocaleString()}
              </option>
            ))}
          </select>
          <button
            type="button"
            disabled={running}
            onClick={() =>
              void perform(
                () => services.document.indexExtraction(documentId),
                "This paper's passages are indexed and ready for answers.",
              )
            }
          >
            Update this paper's index
          </button>
        </>
      )}
      <p role="status" aria-live="polite">
        {notice ||
          (status?.job.documentId === documentId ? status.job.message : "")}
      </p>
      <p>
        Previous text stays available. Check important citations: OCR can
        misread columns, equations and tables.
      </p>
    </section>
  );
}
