import {
  ArrowUpRight,
  BookOpen,
  Check,
  FileText,
  MessageSquareText,
  Network,
  Plus,
  Sparkles,
  X,
} from "lucide-react";
import { motion } from "motion/react";
import { useEffect, useState } from "react";

import type { ResearchItem } from "../../domain/research";
import type { PaperSemanticCoverage } from "../../services/contracts";
import { services } from "../../services/runtime";
import { CitationPanel } from "../citations/CitationPanel";
import { assistantDisplayName } from "../../shared/productIdentity";

interface PaperInspectorProps {
  paper: ResearchItem;
  seedTitles?: string[];
  collections: Map<string, string>;
  selected: boolean;
  relatedPapers: ResearchItem[];
  onClose: () => void;
  onOpenReader: () => void;
  onAddToSelection: () => void;
  onAsk: () => void;
  onSelectRelated: (paperId: string) => void;
  onNavigateCitation: (paperId: string) => void;
  onIndexCitations: () => Promise<void>;
  semanticAvailable: boolean;
}

export function PaperInspector({
  paper,
  seedTitles = [],
  collections,
  selected,
  relatedPapers,
  onClose,
  onOpenReader,
  onAddToSelection,
  onAsk,
  onSelectRelated,
  onNavigateCitation,
  onIndexCitations,
  semanticAvailable,
}: PaperInspectorProps) {
  const [semanticCoverage, setSemanticCoverage] =
    useState<PaperSemanticCoverage | null>(null);

  useEffect(() => {
    let active = true;
    services.semantic
      .coverage(paper.id)
      .then((coverage) => {
        if (active) setSemanticCoverage(coverage);
      })
      .catch(() => {
        if (active) setSemanticCoverage(null);
      });
    return () => {
      active = false;
    };
  }, [paper.id]);
  const collectionNames = paper.collectionIds
    .map((id) => collections.get(id))
    .filter(Boolean);
  const pdfAttachment = paper.attachments.find(
    (attachment) => attachment.mediaType === "application/pdf",
  );
  const isDemo = paper.provenance.sourceId === "graphite-demo";

  return (
    <motion.aside
      className="paper-inspector"
      initial={{ x: 36, opacity: 0 }}
      animate={{ x: 0, opacity: 1 }}
      exit={{ x: 36, opacity: 0 }}
      transition={{ type: "spring", stiffness: 340, damping: 34 }}
      aria-label="Selected paper inspector"
    >
      <div className="inspector-header">
        <span className="eyebrow">PAPER INSPECTOR</span>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close paper inspector"
        >
          <X size={16} />
        </button>
      </div>

      <div className="paper-identity">
        <div className="paper-kicker">
          <span>RESEARCH PAPER</span>
          <span>{paper.year ?? "No year"}</span>
        </div>
        <h2>{paper.title}</h2>
        <p className="paper-authors">
          {paper.authors.map((author) => author.name).join(", ")}
        </p>
        <p className="paper-venue">{paper.venue ?? "Venue not provided"}</p>
      </div>

      <div className="paper-primary-actions">
        <button
          type="button"
          className="primary-action"
          onClick={onOpenReader}
          disabled={!pdfAttachment}
          title={
            pdfAttachment
              ? "Open the local PDF reader"
              : "No PDF attachment is recorded"
          }
        >
          <BookOpen size={16} />
          {pdfAttachment ? "Open paper" : "PDF unavailable"}
          {pdfAttachment && <ArrowUpRight size={14} />}
        </button>
        <button
          type="button"
          className={
            selected ? "secondary-action is-selected" : "secondary-action"
          }
          onClick={onAddToSelection}
        >
          {selected ? <Check size={15} /> : <Plus size={15} />}
          {selected ? "In selection" : "Add"}
        </button>
      </div>

      {seedTitles.length > 0 && (
        <details className="inspector-details">
          <summary>Connected to {seedTitles.length} exploration seeds</summary>
          <ul>
            {seedTitles.map((title) => (
              <li key={title}>{title}</li>
            ))}
          </ul>
        </details>
      )}
      <div className="paper-metrics">
        <div>
          <Network size={14} />
          <strong>{paper.citationCount}</strong>
          <span>cited by locally</span>
        </div>
        <div>
          <FileText size={14} />
          <strong>{paper.notes.length}</strong>
          <span>notes</span>
        </div>
        <div>
          <BookOpen size={14} />
          <strong>{paper.attachments.length}</strong>
          <span>attachments</span>
        </div>
      </div>

      <section className="inspector-section">
        <div className="section-heading">
          <span>ABSTRACT</span>
          <small>{isDemo ? "OPEN-ACCESS DEMO" : "ZOTERO"}</small>
        </div>
        <p className="abstract-text">
          {paper.abstractText ??
            (isDemo
              ? "Open the source article or download its PDF to read the abstract."
              : "No abstract is stored for this Zotero item.")}
        </p>
      </section>

      {semanticCoverage && semanticCoverage.paperStatus !== "active" && (
        <p className="inspector-coverage-note">
          Similarity data is incomplete for this paper. See Data and evidence
          details.
        </p>
      )}
      <details className="inspector-details">
        <summary>Data and evidence details</summary>
        <section
          className="semantic-coverage-strip"
          aria-label="Semantic coverage"
        >
          <div>
            <Sparkles size={14} />
            <span>SEMANTIC REPRESENTATION</span>
          </div>
          <strong>
            {semanticCoverage?.paperStatus ??
              (semanticAvailable ? "checking" : "not indexed")}
          </strong>
          <small>
            {semanticCoverage?.paperRepresentation
              ? `${semanticCoverage.paperRepresentation.replaceAll("-", " ")} / ${semanticCoverage.indexedChunkCount} passages`
              : "Keyword search remains available"}
          </small>
        </section>

        <CitationPanel
          paper={paper}
          onNavigatePaper={onNavigateCitation}
          onIndexPaper={onIndexCitations}
        />

        {paper.attachments.length > 0 && (
          <section className="inspector-section document-section">
            <div className="section-heading">
              <span>DOCUMENT CORPUS</span>
              <small>READ ONLY</small>
            </div>
            {paper.attachments.map((attachment) => (
              <div className="document-row" key={attachment.documentId}>
                <FileText size={14} />
                <div>
                  <strong>{attachment.title}</strong>
                  <small>
                    {attachment.availability.replace("-", " ")} /{" "}
                    {attachment.mediaType}
                  </small>
                </div>
                <span>
                  {attachment.text
                    ? `${attachment.text.status} text`
                    : "not indexed"}
                </span>
              </div>
            ))}
          </section>
        )}
      </details>
      <details className="inspector-details">
        <summary>Tags, collections and notes</summary>
        <section className="inspector-section">
          <div className="section-heading">
            <span>RESEARCH CONTEXT</span>
          </div>
          <div className="tag-list">
            {paper.tags.map((tag) => (
              <span key={tag}>{tag}</span>
            ))}
            {paper.tags.length === 0 && <span>No tags</span>}
          </div>
          <p className="collection-path">
            {collectionNames.join(" / ") || "Unfiled item"}
          </p>
        </section>

        {paper.notes.length > 0 && (
          <section className="inspector-section">
            <div className="section-heading">
              <span>NOTES</span>
              <small>READ ONLY</small>
            </div>
            <div className="note-list">
              {paper.notes.slice(0, 2).map((note) => (
                <div key={note.id}>
                  <strong>{note.title}</strong>
                  <p>{plainNote(note.content)}</p>
                </div>
              ))}
            </div>
          </section>
        )}
      </details>
      <details className="inspector-details">
        <summary>Related library papers</summary>
        <section className="inspector-section related-section">
          <div className="section-heading">
            <span>METADATA NEIGHBOURS</span>
          </div>
          <div className="related-list">
            {relatedPapers.slice(0, 3).map((item) => (
              <button
                type="button"
                key={item.id}
                onClick={() => onSelectRelated(item.id)}
              >
                <span>{item.year}</span>
                <strong>{item.title}</strong>
                <ArrowUpRight size={13} />
              </button>
            ))}
          </div>
        </section>
      </details>
      <div className="inspector-tools">
        <button type="button" onClick={onAsk}>
          <MessageSquareText size={14} />
          Ask {assistantDisplayName}
        </button>
      </div>
    </motion.aside>
  );
}

function plainNote(value: string): string {
  const withoutMarkup = value.replace(/<[^>]+>/g, " ");
  return withoutMarkup.replace(/\s+/g, " ").trim().slice(0, 240);
}
