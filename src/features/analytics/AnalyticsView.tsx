import {
  ArrowUpRight,
  BookOpen,
  CircleDot,
  FileCheck2,
  Network,
} from "lucide-react";
import { motion } from "motion/react";
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";

import { HorizontalScrollbar } from "../../components/HorizontalScrollbar";
import type { ResearchItem, Topic } from "../../domain/research";
import type { AnalyticsSnapshot } from "../../services/contracts";
import { groupPapersByYear } from "./analyticsYears";
import { YearPapersDialog } from "./YearPapersDialog";

interface AnalyticsViewProps {
  analytics: AnalyticsSnapshot;
  topics: Topic[];
  items: ResearchItem[];
  onSelectPaper: (paperId: string) => void;
  provenance: "real-demo" | "zotero-local-cache";
  scopeLabel: string;
  totalItemCount: number;
}

export function AnalyticsView({
  analytics,
  topics,
  items,
  onSelectPaper,
  provenance,
  scopeLabel,
  totalItemCount,
}: AnalyticsViewProps) {
  const [selectedYear, setSelectedYear] = useState<number | null>();
  const yearGroups = useMemo(() => groupPapersByYear(items), [items]);
  const yearChartId = useId();
  const yearChartRef = useRef<HTMLDivElement>(null);
  const [yearScroll, setYearScroll] = useState({
    maximum: 0,
    position: 0,
    viewport: 0,
  });
  const updateYearScroll = useCallback(() => {
    const chart = yearChartRef.current;
    if (!chart) return;
    const maximum = Math.max(0, chart.scrollWidth - chart.clientWidth);
    setYearScroll({
      maximum,
      position: chart.scrollLeft,
      viewport: chart.clientWidth,
    });
  }, []);

  useEffect(() => {
    const chart = yearChartRef.current;
    if (!chart) return;
    const observer = new ResizeObserver(updateYearScroll);
    observer.observe(chart);
    return () => observer.disconnect();
  }, [yearGroups, updateYearScroll]);
  const topicMap = new Map(topics.map((topic) => [topic.id, topic]));
  const maxYearCount = Math.max(
    1,
    ...yearGroups.map((group) => group.papers.length),
  );
  const datedGroups = yearGroups.filter((group) => group.year !== null);
  const firstYear = datedGroups[0]?.year;
  const lastYear = datedGroups.at(-1)?.year;

  return (
    <motion.section
      className="analytics-view"
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.32 }}
    >
      <header className="analytics-header">
        <div>
          <span className="eyebrow">LIBRARY ANALYTICS</span>
          <h1>Shape of the research collection</h1>
          <p>
            Publication years follow the current loaded view. Corpus and
            citation coverage describe the active library.
          </p>
        </div>
        <div className="analytics-summary">
          <span>
            <BookOpen size={14} />
            <strong>{items.length}</strong>
            papers
          </span>
          <span>
            <Network size={14} />
            <strong>
              {
                topics.filter((topic) => topic.origin === "semantic-cluster")
                  .length
              }
            </strong>
            semantic groups
          </span>
          <span>
            <FileCheck2 size={14} />
            <strong>
              {analytics.fullTextCount + analytics.partialTextCount}
            </strong>
            indexed documents
          </span>
          <span>
            <Network size={14} />
            <strong>{analytics.citation.mostCitedLocal.length}</strong>
            locally cited leaders
          </span>
        </div>
      </header>

      <div className="analytics-grid">
        <section className="analytic-panel timeline-panel">
          <div className="analytic-heading">
            <div>
              <span className="eyebrow">PUBLICATION ACTIVITY</span>
              <h2>Papers through time</h2>
            </div>
            <span>
              {firstYear !== undefined
                ? `${firstYear} - ${lastYear}`
                : "No dated papers"}
            </span>
          </div>
          <div
            id={yearChartId}
            ref={yearChartRef}
            className="year-chart"
            aria-label="Papers by publication year"
            onScroll={updateYearScroll}
          >
            {yearGroups.map((group) => {
              const label =
                group.year === null ? "No year" : String(group.year);
              const count = group.papers.length;
              return (
                <button
                  type="button"
                  key={label}
                  title={`${label}: ${count} papers`}
                  aria-label={`${label}: ${count} ${count === 1 ? "paper" : "papers"}`}
                  aria-haspopup="dialog"
                  onClick={() => setSelectedYear(group.year)}
                >
                  <span
                    style={{ height: `${(count / maxYearCount) * 100}%` }}
                  />
                  <small>{label}</small>
                </button>
              );
            })}
            {yearGroups.length === 0 && <p>No papers in the current view.</p>}
          </div>
          <HorizontalScrollbar
            controlsId={yearChartId}
            label="Scroll publication years"
            {...yearScroll}
            onScroll={(position) => {
              if (yearChartRef.current) {
                yearChartRef.current.scrollLeft = position;
                updateYearScroll();
              }
            }}
          />
          <p>
            {scopeLabel}: {items.length} loaded papers of {totalItemCount} in
            the active library. Select a year to inspect its papers.{" "}
            {provenance !== "zotero-local-cache"
              ? "Publication years of the real seismology demo papers."
              : "Publication years are parsed conservatively from Zotero date metadata."}
          </p>
        </section>

        <section className="analytic-panel topic-panel">
          <div className="analytic-heading">
            <div>
              <span className="eyebrow">LIBRARY COMPOSITION</span>
              <h2>Automatic semantic topics</h2>
            </div>
            <CircleDot size={15} />
          </div>
          {!topics.some((topic) => topic.origin === "semantic-cluster") && (
            <p>
              No clear groups available. Topic discovery uses prepared library
              content.
            </p>
          )}
          <div
            className="topic-bars"
            tabIndex={0}
            role="region"
            aria-label="Topic counts"
          >
            {analytics.byTopic.map((metric) => {
              const topic = topicMap.get(metric.topicId);
              return (
                <div key={metric.topicId}>
                  <span>{topic?.shortName ?? metric.topicId}</span>
                  <i>
                    <b
                      style={{
                        width: `${(metric.count / items.length) * 420}%`,
                        background: topic?.color,
                      }}
                    />
                  </i>
                  <strong>{metric.count}</strong>
                </div>
              );
            })}
          </div>
        </section>

        <section className="analytic-panel connected-panel">
          <div className="analytic-heading">
            <div>
              <span className="eyebrow">GRAPH CENTRALITY PROXY</span>
              <h2>Most connected by metadata</h2>
            </div>
            <span>GRAPH LINKS</span>
          </div>
          <div className="connected-list">
            {analytics.mostConnected.map((paper, index) => {
              const topic = topicMap.get(paper.topics[0]);
              return (
                <button
                  type="button"
                  key={paper.id}
                  onClick={() => onSelectPaper(paper.id)}
                >
                  <span>{String(index + 1).padStart(2, "0")}</span>
                  <i style={{ background: topic?.color }} />
                  <div>
                    <strong>{paper.title}</strong>
                    <small>
                      {paper.authors[0]?.name ?? "Unknown creator"} /{" "}
                      {paper.year ?? "No year"}
                    </small>
                  </div>
                  <b>{paper.citationCount}</b>
                  <ArrowUpRight size={14} />
                </button>
              );
            })}
          </div>
        </section>

        <section className="analytic-panel matrix-panel">
          <div className="analytic-heading">
            <div>
              <span className="eyebrow">METADATA COMPLETENESS</span>
              <h2>Fields available for research workflows</h2>
            </div>
            <span>ACTIVE LIBRARY</span>
          </div>
          <div className="completeness-bars">
            {metadataCompleteness(items).map((field) => (
              <div key={field.label}>
                <span>{field.label}</span>
                <i>
                  <b style={{ width: `${field.ratio * 100}%` }} />
                </i>
                <strong>{Math.round(field.ratio * 100)}%</strong>
              </div>
            ))}
          </div>
          <p>
            Missing metadata fields remain null; Graphite does not infer
            replacements during M3.
          </p>
        </section>

        <section className="analytic-panel citation-ranking-panel">
          <div className="analytic-heading">
            <div>
              <span className="eyebrow">LOCAL CITATION NETWORK</span>
              <h2>Most cited within your library</h2>
            </div>
            <span>LOCAL SCOPE</span>
          </div>
          <div className="citation-ranking-list">
            {analytics.citation.mostCitedLocal.map((metric, index) => (
              <button
                type="button"
                key={metric.itemId}
                onClick={() => onSelectPaper(metric.itemId)}
              >
                <span>{String(index + 1).padStart(2, "0")}</span>
                <strong>{metric.title}</strong>
                <b>{metric.count}</b>
                <ArrowUpRight size={13} />
              </button>
            ))}
            {analytics.citation.mostCitedLocal.length === 0 && (
              <p>Index bibliographies to establish local incoming citations.</p>
            )}
          </div>
          <p>
            Counts describe only indexed papers in Graphite. They are not global
            scholarly citation counts or impact measures.
          </p>
        </section>

        <section className="analytic-panel external-ranking-panel">
          <div className="analytic-heading">
            <div>
              <span className="eyebrow">MISSING FROM LIBRARY</span>
              <h2>Frequently referenced external works</h2>
            </div>
            <BookOpen size={15} />
          </div>
          <div className="external-ranking-list">
            {analytics.citation.frequentlyReferencedExternal.map((work) => (
              <div key={work.workId}>
                <strong>{work.title}</strong>
                <small>
                  {work.year ?? "Year unresolved"}
                  {work.doi ? ` / ${work.doi}` : " / no DOI parsed"}
                </small>
                <span>{work.referencedByCount} local papers</span>
              </div>
            ))}
          </div>
          <p>
            These works come from local bibliography evidence; Graphite performs
            no online recommendation or automatic Zotero import.
          </p>
        </section>

        <section className="analytic-panel citation-coverage-panel">
          <div className="analytic-heading">
            <div>
              <span className="eyebrow">REFERENCE RESOLUTION</span>
              <h2>Local identity coverage</h2>
            </div>
            <strong>
              {Math.round(analytics.citation.resolutionCoverage * 100)}%
            </strong>
          </div>
          <div className="citation-resolution-scale">
            <span
              style={{
                width: `${analytics.citation.resolutionCoverage * 100}%`,
              }}
            />
          </div>
          <p>
            Exact identifiers and strong multi-field metadata matches are
            resolved. Probable or ambiguous candidates remain outside the
            authoritative citation graph.
          </p>
        </section>

        <section className="analytic-panel corpus-panel">
          <div className="analytic-heading">
            <div>
              <span className="eyebrow">DOCUMENT CORPUS</span>
              <h2>Local reading coverage</h2>
            </div>
            <FileCheck2 size={15} />
          </div>
          <div className="corpus-metrics">
            <span>
              <strong>{analytics.documentCount}</strong> attachments
            </span>
            <span>
              <strong>{analytics.resolvedPdfCount}</strong> resolved PDFs
            </span>
            <span>
              <strong>{analytics.fullTextCount}</strong> full text
            </span>
            <span>
              <strong>{analytics.partialTextCount}</strong> partial text
            </span>
            <span>
              <strong>{analytics.indexedPages.toLocaleString()}</strong> indexed
              pages
            </span>
          </div>
          <p>
            {provenance === "real-demo"
              ? "Downloaded demo PDFs supply locally prepared text. Source PDFs stay unchanged."
              : "Zotero-indexed text is copied into Graphite's disposable derived corpus. Zotero remains untouched."}
          </p>
        </section>
      </div>
      {selectedYear !== undefined && (
        <YearPapersDialog
          year={selectedYear}
          papers={
            yearGroups.find((group) => group.year === selectedYear)?.papers ??
            []
          }
          onClose={() => setSelectedYear(undefined)}
          onSelectPaper={onSelectPaper}
        />
      )}
    </motion.section>
  );
}

function metadataCompleteness(items: ResearchItem[]) {
  const total = Math.max(1, items.length);
  const ratio = (predicate: (item: ResearchItem) => boolean) =>
    items.filter(predicate).length / total;
  return [
    { label: "Year", ratio: ratio((item) => item.year !== null) },
    { label: "Venue", ratio: ratio((item) => item.venue !== null) },
    { label: "Abstract", ratio: ratio((item) => item.abstractText !== null) },
    { label: "Creator", ratio: ratio((item) => item.authors.length > 0) },
    { label: "Tags", ratio: ratio((item) => item.tags.length > 0) },
  ];
}
