import {
  ArrowDownLeft,
  ArrowRight,
  BookMarked,
  CircleHelp,
  ExternalLink,
  LoaderCircle,
  Network,
  ShieldCheck,
} from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useState } from "react";

import type { ResearchItem } from "../../domain/research";
import { services } from "../../services/runtime";
import type {
  BibliographicReferenceView,
  CitationItemPage,
  CoCitedWorkPage,
  ReferenceFilter,
  ReferencePage,
  StructuralPaperPage,
} from "../../services/contracts";

interface CitationPanelProps {
  paper: ResearchItem;
  onNavigatePaper: (paperId: string) => void;
  onIndexPaper: () => Promise<void>;
}

type CitationTab = "references" | "cited-by" | "structure";

const referenceFilters: Array<{ id: ReferenceFilter; label: string }> = [
  { id: "all", label: "All" },
  { id: "local", label: "In Graphite" },
  { id: "external", label: "External" },
  { id: "ambiguous", label: "Review" },
  { id: "unresolved", label: "Unresolved" },
];

export function CitationPanel({
  paper,
  onNavigatePaper,
  onIndexPaper,
}: CitationPanelProps) {
  const [tab, setTab] = useState<CitationTab>("references");
  const [filter, setFilter] = useState<ReferenceFilter>("all");
  const [referencePage, setReferencePage] = useState<ReferencePage | null>(
    null,
  );
  const [citedBy, setCitedBy] = useState<CitationItemPage | null>(null);
  const [shared, setShared] = useState<StructuralPaperPage | null>(null);
  const [coCited, setCoCited] = useState<CoCitedWorkPage | null>(null);
  const [expandedReferenceId, setExpandedReferenceId] = useState<string | null>(
    null,
  );
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(
    async (nextFilter: ReferenceFilter, offset = 0, append = false) => {
      setLoading(true);
      setError(null);
      try {
        const [references, incoming, coupling, coCitation] = await Promise.all([
          services.citations.references(paper.id, nextFilter, 30, offset),
          services.citations.citedBy(paper.id, 20, 0),
          services.citations.sharedReferences(paper.id, 12),
          services.citations.coCited(paper.id, 12),
        ]);
        setReferencePage((current) =>
          append && current
            ? {
                ...references,
                references: [...current.references, ...references.references],
              }
            : references,
        );
        setCitedBy(incoming);
        setShared(coupling);
        setCoCited(coCitation);
      } catch (reason: unknown) {
        setError(reason instanceof Error ? reason.message : String(reason));
      } finally {
        setLoading(false);
      }
    },
    [paper.id],
  );

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => {
      void load(filter, 0, false);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [filter, load]);

  const chooseFilter = (nextFilter: ReferenceFilter) => {
    setFilter(nextFilter);
    setExpandedReferenceId(null);
  };

  const review = async (
    referenceId: string,
    decision: "accept" | "reject" | "unresolved",
    targetItemId?: string,
  ) => {
    await services.citations.review(referenceId, decision, targetItemId);
    await load(filter, 0, false);
  };

  const activeReferencePage =
    referencePage?.coverage.itemId === paper.id ? referencePage : null;
  const coverage = activeReferencePage?.coverage;

  return (
    <section className="citation-panel" aria-label="Citation intelligence">
      <div className="citation-tabs" role="tablist">
        <button
          type="button"
          className={tab === "references" ? "is-active" : ""}
          onClick={() => setTab("references")}
        >
          References
          <span>{coverage?.totalReferences ?? "-"}</span>
        </button>
        <button
          type="button"
          className={tab === "cited-by" ? "is-active" : ""}
          onClick={() => setTab("cited-by")}
        >
          Cited by
          <span>{coverage?.citedByLocal ?? "-"}</span>
        </button>
        <button
          type="button"
          className={tab === "structure" ? "is-active" : ""}
          onClick={() => setTab("structure")}
        >
          Structure
          <Network size={12} />
        </button>
      </div>

      {loading && !referencePage ? (
        <div className="citation-loading">
          <LoaderCircle size={15} className="is-spinning" />
          Reading local citation index
        </div>
      ) : error ? (
        <div className="citation-unavailable">
          <CircleHelp size={16} />
          <div>
            <strong>Citation index unavailable</strong>
            <span>{error}</span>
          </div>
          <button type="button" onClick={() => void load(filter)}>
            Retry
          </button>
        </div>
      ) : tab === "references" &&
        coverage?.extractionStatus === "not-indexed" ? (
        <div className="citation-unavailable">
          <BookMarked size={16} />
          <div>
            <strong>References not indexed</strong>
            <span>Prioritize this paper in the local extraction queue.</span>
          </div>
          <button
            type="button"
            onClick={() => {
              void onIndexPaper().then(() => load(filter));
            }}
          >
            Index paper
          </button>
        </div>
      ) : tab === "references" &&
        coverage &&
        coverage.extractionStatus !== "references-extracted" ? (
        <div className="citation-unavailable">
          <CircleHelp size={16} />
          <div>
            <strong>Reference index is non-authoritative</strong>
            <span>
              {extractionStatusLabel(coverage.extractionStatus)} Automatic
              citation edges were not created for this document.
            </span>
          </div>
        </div>
      ) : tab === "references" ? (
        <div className="reference-browser">
          <CitationCoverageStrip page={activeReferencePage} />
          <div className="reference-filters">
            {referenceFilters.map((candidate) => (
              <button
                type="button"
                key={candidate.id}
                className={filter === candidate.id ? "is-active" : ""}
                onClick={() => chooseFilter(candidate.id)}
              >
                {candidate.label}
              </button>
            ))}
          </div>
          <div className="reference-list">
            {activeReferencePage?.references.map((reference) => (
              <ReferenceRow
                key={reference.id}
                reference={reference}
                expanded={expandedReferenceId === reference.id}
                onExpand={() =>
                  setExpandedReferenceId((current) =>
                    current === reference.id ? null : reference.id,
                  )
                }
                onNavigatePaper={onNavigatePaper}
                onReview={review}
              />
            ))}
            {activeReferencePage?.references.length === 0 && (
              <div className="reference-empty">
                No references in this resolution category.
              </div>
            )}
          </div>
          {activeReferencePage &&
            activeReferencePage.references.length <
              activeReferencePage.total && (
              <button
                type="button"
                className="citation-load-more"
                onClick={() =>
                  void load(filter, activeReferencePage.references.length, true)
                }
              >
                Showing {activeReferencePage.references.length} of{" "}
                {activeReferencePage.total}
                <span>Load more</span>
              </button>
            )}
        </div>
      ) : tab === "cited-by" ? (
        <div className="cited-by-list">
          <p>
            Incoming links are limited to papers with trustworthy bibliography
            evidence in your indexed Graphite corpus.
          </p>
          {citedBy?.items.map((item) => (
            <button
              type="button"
              key={item.id}
              onClick={() => onNavigatePaper(item.id)}
            >
              <ArrowDownLeft size={13} />
              <span>
                <strong>{item.title}</strong>
                <small>
                  {item.firstAuthor ?? "Unknown creator"} / {item.year ?? "-"}
                </small>
              </span>
              <ExternalLink size={12} />
            </button>
          ))}
          {citedBy?.items.length === 0 && (
            <div className="reference-empty">
              No incoming citations detected in your indexed Graphite corpus.
            </div>
          )}
        </div>
      ) : (
        <div className="structural-links">
          <div className="structural-block">
            <header>
              <span>SHARED REFERENCES</span>
            </header>
            {shared?.links.map((link) => (
              <button
                type="button"
                key={link.item.id}
                onClick={() => onNavigatePaper(link.item.id)}
              >
                <strong>{link.item.title}</strong>
                <span>{link.count} shared references</span>
              </button>
            ))}
            {shared?.links.length === 0 && (
              <small>No coupling above the two-reference threshold.</small>
            )}
          </div>
          <div className="structural-block">
            <header>
              <span>LOCAL-CORPUS CO-CITATION</span>
            </header>
            {coCited?.links.map((link, index) => (
              <button
                type="button"
                key={link.localItem?.id ?? link.externalWork?.id ?? index}
                onClick={() => {
                  if (link.localItem) onNavigatePaper(link.localItem.id);
                }}
                disabled={!link.localItem}
              >
                <strong>
                  {link.localItem?.title ??
                    link.externalWork?.title ??
                    "Unresolved referenced work"}
                </strong>
                <span>{link.label}</span>
              </button>
            ))}
          </div>
        </div>
      )}
    </section>
  );
}

function CitationCoverageStrip({ page }: { page: ReferencePage | null }) {
  const coverage = page?.coverage;
  if (!coverage) return null;
  return (
    <div className="citation-coverage-strip">
      <span>
        <strong>{coverage.resolvedLocal}</strong> in Graphite
      </span>
      <span>
        <strong>{coverage.external}</strong> external
      </span>
      <span className={coverage.ambiguous > 0 ? "has-warning" : ""}>
        <strong>{coverage.ambiguous}</strong> review
      </span>
      <span>
        <strong>{coverage.unresolved}</strong> unresolved
      </span>
    </div>
  );
}

interface ReferenceRowProps {
  reference: BibliographicReferenceView;
  expanded: boolean;
  onExpand: () => void;
  onNavigatePaper: (paperId: string) => void;
  onReview: (
    referenceId: string,
    decision: "accept" | "reject" | "unresolved",
    targetItemId?: string,
  ) => Promise<void>;
}

function ReferenceRow({
  reference,
  expanded,
  onExpand,
  onNavigatePaper,
  onReview,
}: ReferenceRowProps) {
  const displayTitle =
    reference.matchedItem?.title ??
    reference.externalWork?.title ??
    reference.parsedTitle ??
    reference.rawText;
  return (
    <motion.article
      layout
      className={`reference-row status-${reference.resolutionStatus}`}
    >
      <button type="button" className="reference-summary" onClick={onExpand}>
        <span className="reference-ordinal">
          {String(reference.ordinal).padStart(2, "0")}
        </span>
        <span>
          <strong>{displayTitle}</strong>
          <small>
            {reference.firstAuthor ?? "Creator unresolved"} /{" "}
            {reference.parsedYear ?? "-"}
          </small>
        </span>
        <i>{resolutionLabel(reference)}</i>
      </button>
      <AnimatePresence initial={false}>
        {expanded && (
          <motion.div
            className="reference-evidence"
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: 0.2 }}
          >
            <div className="evidence-heading">
              <ShieldCheck size={13} />
              <span>RAW BIBLIOGRAPHY EVIDENCE</span>
            </div>
            <p>{reference.rawText}</p>
            <dl>
              <div>
                <dt>Resolution</dt>
                <dd>{reference.confidenceCategory}</dd>
              </div>
              <div>
                <dt>Method</dt>
                <dd>{reference.resolutionMethod.replaceAll("-", " ")}</dd>
              </div>
              {reference.normalizedDoi && (
                <div>
                  <dt>Normalized DOI</dt>
                  <dd>{reference.normalizedDoi}</dd>
                </div>
              )}
              <div>
                <dt>Location</dt>
                <dd>
                  {reference.pageNumber
                    ? `Page ${reference.pageNumber}`
                    : "Text range only; page unavailable"}
                </dd>
              </div>
            </dl>
            {reference.resolutionEvidence.map((item) => (
              <span className="evidence-reason" key={item}>
                {item}
              </span>
            ))}
            {reference.matchedItem && (
              <button
                type="button"
                className="reference-open-local"
                onClick={() => onNavigatePaper(reference.matchedItem?.id ?? "")}
              >
                Open local paper <ArrowRight size={12} />
              </button>
            )}
            {reference.candidates.length > 0 && (
              <div className="resolution-review">
                <span>REVIEW CANDIDATES</span>
                {reference.candidates.map((candidate) => (
                  <div key={candidate.item.id}>
                    <button
                      type="button"
                      onClick={() =>
                        void onReview(reference.id, "accept", candidate.item.id)
                      }
                    >
                      <strong>{candidate.item.title}</strong>
                      <small>{candidate.evidence.join(" / ")}</small>
                      <span>Accept match</span>
                    </button>
                  </div>
                ))}
                <button
                  type="button"
                  className="mark-external"
                  onClick={() => void onReview(reference.id, "reject")}
                >
                  Keep as external work
                </button>
                <button
                  type="button"
                  className="mark-unresolved"
                  onClick={() => void onReview(reference.id, "unresolved")}
                >
                  Keep unresolved
                </button>
              </div>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </motion.article>
  );
}

function resolutionLabel(reference: BibliographicReferenceView): string {
  if (reference.matchedItem) return "IN GRAPHITE";
  if (reference.resolutionStatus === "ambiguous") return "REVIEW";
  if (reference.resolutionStatus === "external") return "EXTERNAL";
  if (reference.resolutionStatus === "probable") return "PROBABLE";
  return "UNRESOLVED";
}

function extractionStatusLabel(status: string): string {
  switch (status) {
    case "partial-source-text":
      return "Zotero supplied partial indexed text.";
    case "low-confidence-section":
      return "A possible bibliography had too little structural evidence.";
    case "no-reference-section":
      return "No trustworthy bibliography section was detected.";
    case "failed":
      return "Local reference extraction failed.";
    default:
      return "Reference extraction is unavailable.";
  }
}
