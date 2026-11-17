import {
  ArrowRight,
  BookOpenText,
  ExternalLink,
  FileSearch,
  ShieldCheck,
  X,
} from "lucide-react";
import { motion } from "motion/react";

import type { GraphNode } from "../../domain/graph";
import type {
  RelationshipEvidence,
  SemanticRelationshipEvidence,
} from "../../services/contracts";

interface RelationshipInspectorProps {
  evidence: RelationshipEvidence | SemanticRelationshipEvidence | null;
  externalNode: GraphNode | null;
  seedTitles?: string[];
  onClose: () => void;
  onNavigatePaper: (paperId: string) => void;
}

export function RelationshipInspector({
  evidence,
  externalNode,
  seedTitles = [],
  onClose,
  onNavigatePaper,
}: RelationshipInspectorProps) {
  const isSemantic = evidence?.relation === "semantically-similar";
  return (
    <motion.aside
      className="relationship-inspector"
      initial={{ x: 34, opacity: 0 }}
      animate={{ x: 0, opacity: 1 }}
      exit={{ x: 34, opacity: 0 }}
      transition={{ type: "spring", stiffness: 340, damping: 34 }}
      aria-label={
        evidence ? "Why are these connected?" : "External referenced work"
      }
    >
      <header>
        <div>
          <span className="eyebrow">
            {evidence ? "RELATIONSHIP EVIDENCE" : "REFERENCED WORK"}
          </span>
          <strong>
            {evidence ? "Why are these connected?" : "Not in Zotero"}
          </strong>
        </div>
        <button type="button" onClick={onClose} aria-label="Close inspector">
          <X size={15} />
        </button>
      </header>

      {seedTitles.length > 0 && (
        <details>
          <summary>Connected to {seedTitles.length} exploration seeds</summary>
          <ul>
            {seedTitles.map((title, index) => (
              <li key={index}>{title}</li>
            ))}
          </ul>
        </details>
      )}
      {evidence ? (
        <>
          <div className="edge-direction">
            <button
              type="button"
              onClick={() => onNavigatePaper(evidence.source.id)}
            >
              <span>SOURCE</span>
              <strong>{evidence.source.title}</strong>
            </button>
            {evidence.relation === "citation" ? (
              <ArrowRight size={17} />
            ) : (
              <span>and</span>
            )}
            <button
              type="button"
              onClick={() => onNavigatePaper(evidence.target.id)}
            >
              <span>
                {evidence.relation === "citation"
                  ? "CITED WORK"
                  : "CONNECTED PAPER"}
              </span>
              <strong>{evidence.target.title}</strong>
            </button>
          </div>
          {isSemantic ? (
            <section className="semantic-edge-evidence">
              <div>
                <ShieldCheck size={14} />
                <span>LOCAL VECTOR EVIDENCE</span>
              </div>
              <strong>{evidence.similarityLabel}</strong>
              <p>
                Based on locally indexed titles and abstracts. Similar wording
                does not establish agreement, quality or scientific equivalence.
              </p>
              <details>
                <summary>Similarity details</summary>
                <dl>
                  <div>
                    <dt>Similarity</dt>
                    <dd>{evidence.similarityScore.toFixed(3)}</dd>
                  </div>
                  <div>
                    <dt>Metric</dt>
                    <dd>{evidence.similarityMetric}</dd>
                  </div>
                  <div>
                    <dt>Representations</dt>
                    <dd>
                      {evidence.sourceRepresentation.replaceAll("-", " ")} /{" "}
                      {evidence.targetRepresentation.replaceAll("-", " ")}
                    </dd>
                  </div>
                  <div>
                    <dt>Profile</dt>
                    <dd>{evidence.embeddingProfile.modelIdentifier}</dd>
                  </div>
                </dl>
              </details>
              <p>
                This edge is derived from local paper vectors. It is not a
                citation claim and does not change Zotero.
              </p>
            </section>
          ) : (
            <p className="relationship-scope">{evidence.evidenceLabel}</p>
          )}
          {!isSemantic && evidence.details && (
            <section className="connection-details">
              <strong>
                {evidence.relation === "bibliographic-coupling"
                  ? "Works cited by both papers"
                  : evidence.relation === "co-cited"
                    ? "Local papers citing both works"
                    : "Supporting evidence"}
              </strong>
              {evidence.details.length ? (
                <ul>
                  {evidence.details.map((detail, index) => (
                    <li key={index}>{detail}</li>
                  ))}
                </ul>
              ) : (
                <p>No further details are available in the current index.</p>
              )}
            </section>
          )}
          {!isSemantic &&
            evidence.evidence.map((item) => (
              <section className="edge-evidence-record" key={item.referenceId}>
                <div>
                  <ShieldCheck size={14} />
                  <span>REFERENCE {item.ordinal}</span>
                  <strong>{item.confidenceCategory}</strong>
                </div>
                <blockquote>{item.rawText}</blockquote>
                <dl>
                  <div>
                    <dt>Resolved by</dt>
                    <dd>{item.resolutionMethod.replaceAll("-", " ")}</dd>
                  </div>
                  <div>
                    <dt>Source</dt>
                    <dd>{item.sourceDocumentTitle}</dd>
                  </div>
                  <div>
                    <dt>Location</dt>
                    <dd>
                      {item.pageNumber
                        ? `Page ${item.pageNumber}`
                        : "No reliable page mapping"}
                    </dd>
                  </div>
                  <div>
                    <dt>Algorithms</dt>
                    <dd>
                      extractor {item.extractorVersion} / resolver{" "}
                      {item.resolverVersion}
                    </dd>
                  </div>
                </dl>
              </section>
            ))}
        </>
      ) : externalNode ? (
        <div className="external-work-inspection">
          <div className="external-work-glyph">
            <BookOpenText size={21} />
          </div>
          <h2>{externalNode.label}</h2>
          <p>
            {externalNode.detail || "Bibliographic metadata is incomplete."}
          </p>
          <div>
            <FileSearch size={14} />
            This outlined node is known only from bibliography evidence in your
            local corpus. Graphite has not matched it to a Zotero item.
          </div>
          <button type="button" disabled>
            <ExternalLink size={13} />
            Online enrichment is disabled
          </button>
        </div>
      ) : null}
    </motion.aside>
  );
}
