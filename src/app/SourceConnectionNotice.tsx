import { RefreshCw, Unlink, X } from "lucide-react";
import { useState } from "react";

import type { SourceDiagnostics } from "../services/contracts";

interface SourceConnectionNoticeProps {
  source: SourceDiagnostics;
  syncing: boolean;
  syncError: string | null;
  onRetry: () => void;
  onUseDemo: () => void;
}

interface ZoteroConnectionGuidance {
  title: string;
  message: string;
}

export function SourceConnectionNotice({
  source,
  syncing,
  syncError,
  onRetry,
  onUseDemo,
}: SourceConnectionNoticeProps) {
  const [dismissed, setDismissed] = useState(false);
  const error = syncError ?? source.lastError;
  if (
    dismissed ||
    source.mode !== "zotero" ||
    syncing ||
    !error ||
    !["error", "offline-cache", "incompatible"].includes(source.health)
  ) {
    return null;
  }

  const guidance = describeZoteroConnectionIssue(
    error,
    source.health === "offline-cache",
  );

  return (
    <aside
      className="source-connection-notice"
      role="status"
      aria-live="polite"
      aria-atomic="true"
    >
      <Unlink className="source-connection-notice__icon" size={17} />
      <div className="source-connection-notice__copy">
        <strong>{guidance.title}</strong>
        <p>{guidance.message}</p>
        <small>
          Graphite remains local and has not changed your Zotero library.
        </small>
      </div>
      <button
        type="button"
        className="source-connection-notice__dismiss"
        aria-label="Dismiss Zotero connection notice"
        onClick={() => setDismissed(true)}
      >
        <X size={14} />
      </button>
      <div className="source-connection-notice__actions">
        <button type="button" onClick={onRetry}>
          <RefreshCw size={13} />
          Retry Zotero
        </button>
        <button type="button" onClick={onUseDemo}>
          Use demo data
        </button>
      </div>
    </aside>
  );
}

function describeZoteroConnectionIssue(
  error: string,
  hasCachedLibrary: boolean,
): ZoteroConnectionGuidance {
  if (error.includes("local API access is disabled")) {
    return {
      title: "Zotero local access is disabled",
      message:
        "Enable 'Allow other applications on this computer to communicate with Zotero' in Zotero's Advanced settings, then retry.",
    };
  }
  if (error.includes("not reachable at 127.0.0.1:23119")) {
    return {
      title: "Zotero is not available",
      message: hasCachedLibrary
        ? "Start Zotero, then retry. Your previously synchronized library remains available while Zotero is offline."
        : "Start Zotero, then retry. If it is already open, confirm that local API access is enabled in Zotero's Advanced settings.",
    };
  }
  if (error.includes("Zotero-Server-ID") || error.includes("modern Zotero")) {
    return {
      title: "This Zotero instance could not be verified",
      message:
        "Graphite requires a current Zotero Local API with server identity support. Update Zotero or review Data Source for details, then retry.",
    };
  }
  return {
    title: "Zotero could not be synchronized",
    message: hasCachedLibrary
      ? "Your cached library remains available. Check Zotero and retry when the local connection is ready."
      : "Check that Zotero is running and local API access is enabled, then retry.",
  };
}
