import type { SemanticDiagnostics } from "../../services/contracts";

type PreparationDiagnostics = Pick<
  SemanticDiagnostics,
  | "modelInstalled"
  | "indexedChunks"
  | "chunkedDocuments"
  | "eligibleDocuments"
  | "indexing"
>;

/** Explain the next preparation step without disabling lexical discovery. */
export function agentPreparation(
  semantic: PreparationDiagnostics,
  answerModelMissing: boolean,
  demo: boolean,
  availableTextDocuments: number,
  externalNeedsCheck = false,
): { title: string; message: string; action: "library" | "model" } | null {
  if (demo && availableTextDocuments === 0) {
    return {
      title: "Prepare the real demo papers",
      message:
        "Choose Prepare library for AI to download the demo or open its offline ZIP, then prepare document evidence.",
      action: "library",
    };
  }
  if (!semantic.modelInstalled) {
    return {
      title: "Embedding model needed",
      message:
        "Install the embedding (search) model in library setup, then resume preparation. Models are never downloaded automatically.",
      action: "library",
    };
  }
  if (semantic.indexedChunks === 0) {
    return {
      title: semantic.indexing
        ? "Preparing document passages"
        : "No document passages are indexed yet",
      message:
        availableTextDocuments === 0
          ? "Choose Prepare library for AI to prepare available Zotero text and PDFs. An answer model alone cannot supply document evidence."
          : "Choose Prepare library for AI in library setup. It continues indexing automatically; wait for document passages before asking scientific questions.",
      action: "library",
    };
  }
  if (answerModelMissing) {
    return {
      title: "Answer model needed",
      message:
        "Your library has document passages. Open TYRION Setup to download an answer model or choose one already installed.",
      action: "model",
    };
  }
  if (externalNeedsCheck) {
    return {
      title: "Check your local AI app",
      message:
        "Open TYRION Setup and check the connection to your configured local model before asking a question.",
      action: "model",
    };
  }
  if (semantic.chunkedDocuments < semantic.eligibleDocuments) {
    return {
      title: `${semantic.chunkedDocuments} of ${semantic.eligibleDocuments} documents prepared`,
      message:
        "Coverage is partial. Resume preparation in library setup. Questions can use prepared papers; other papers may have no usable evidence.",
      action: "library",
    };
  }
  return null;
}
