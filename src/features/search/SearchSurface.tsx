import {
  ArrowRight,
  Command,
  CornerDownLeft,
  Search,
  Sparkles,
  X,
} from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useRef, useState } from "react";

import type { ResearchItem } from "../../domain/research";
import { services } from "../../services/runtime";
import type {
  CitationSearchFilter,
  SearchResult,
  SearchMode,
  SemanticDiagnostics,
} from "../../services/contracts";

interface SearchSurfaceProps {
  initialQuery?: string;
  open: boolean;
  selectedIds: string[];
  onTogglePaper: (paper: ResearchItem) => void;
  onReviewSelection: () => void;
  onClose: () => void;
  onFocusResults: (resultIds: string[], selectedId?: string) => void;
  onSelectPaper: (id: string) => void;
  sourceMode: "demo" | "zotero";
  semantic: SemanticDiagnostics;
}

const exampleQueries = [
  "surface wave dispersion crust",
  "ambient noise tomography",
  "Bayesian inversion",
];

const citationFilters: Array<{
  id: CitationSearchFilter;
  label: string;
}> = [
  { id: "all", label: "All papers" },
  { id: "references-indexed", label: "References indexed" },
  { id: "cited-by-local", label: "Cited locally" },
  { id: "has-external", label: "External references" },
  { id: "needs-review", label: "Needs citation review" },
];

export function SearchSurface({
  initialQuery = "",
  open,
  selectedIds,
  onTogglePaper,
  onReviewSelection,
  onClose,
  onFocusResults,
  onSelectPaper,
  sourceMode,
  semantic,
}: SearchSurfaceProps) {
  const [query, setQuery] = useState(initialQuery);
  const [results, setResults] = useState<SearchResult[]>([]);
  const [total, setTotal] = useState(0);
  const [corpusStatus, setCorpusStatus] = useState("not-indexed");
  const [semanticCoverage, setSemanticCoverage] = useState({
    indexed: 0,
    eligible: 0,
  });
  const [searchError, setSearchError] = useState<string | null>(null);
  const [mode, setMode] = useState<SearchMode>(
    semantic.modelInstalled && semantic.indexedPapers > 0
      ? "hybrid"
      : "keyword",
  );
  const [activeIndex, setActiveIndex] = useState(0);
  const [citationFilter, setCitationFilter] =
    useState<CitationSearchFilter>("all");
  const searchGeneration = useRef(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!open) return;
    previousFocusRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const frame = requestAnimationFrame(() => inputRef.current?.focus());
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
        return;
      }
      if (event.key !== "Tab") {
        return;
      }

      const focusable = dialogRef.current?.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input:not(:disabled), [tabindex]:not([tabindex="-1"])',
      );
      if (!focusable || focusable.length === 0) {
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      cancelAnimationFrame(frame);
      document.removeEventListener("keydown", handleKeyDown);
      previousFocusRef.current?.focus();
    };
  }, [onClose, open]);

  useEffect(() => {
    searchGeneration.current++;
    if (!open) return;
    let active = true;
    const timer = window.setTimeout(() => {
      services.search
        .search(query, 20, 0, citationFilter, mode)
        .then((page) => {
          if (active) {
            setResults(page.results);
            setTotal(page.total);
            setCorpusStatus(page.corpusStatus);
            setSemanticCoverage({
              indexed: page.semanticIndexedPapers,
              eligible: page.semanticEligiblePapers,
            });
            setSearchError(null);
            setActiveIndex(0);
          }
        })
        .catch((reason: unknown) => {
          if (!active) return;
          setResults([]);
          setTotal(0);
          setSearchError(
            reason instanceof Error ? reason.message : String(reason),
          );
        });
    }, 90);

    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [citationFilter, mode, open, query]);

  const chooseResult = (result: SearchResult) => {
    onSelectPaper(result.item.id);
    onClose();
  };

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          className="search-backdrop"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.16 }}
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) onClose();
          }}
        >
          <motion.div
            ref={dialogRef}
            className="search-surface"
            role="dialog"
            aria-modal="true"
            aria-label="Search research library"
            initial={{ opacity: 0, y: -18, scale: 0.985 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -12, scale: 0.99 }}
            transition={{ type: "spring", stiffness: 370, damping: 32 }}
          >
            <div className="search-input-row">
              <Search size={19} />
              <input
                ref={inputRef}
                name="research-library-query"
                autoComplete="off"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Escape") onClose();
                  if (event.key === "ArrowDown") {
                    event.preventDefault();
                    setActiveIndex((index) =>
                      Math.min(results.length - 1, index + 1),
                    );
                  }
                  if (event.key === "ArrowUp") {
                    event.preventDefault();
                    setActiveIndex((index) => Math.max(0, index - 1));
                  }
                  if (event.key === "Enter" && results[activeIndex]) {
                    chooseResult(results[activeIndex]);
                  }
                }}
                placeholder="Search titles, creators, tags, and abstracts"
                aria-label="Research library query"
              />
              <span className="search-mode">
                <Sparkles size={12} />
                {sourceMode === "demo" ? "DEMO CORPUS" : "LOCAL RETRIEVAL"}
              </span>
              <button type="button" onClick={onClose} aria-label="Close search">
                <X size={16} />
              </button>
            </div>

            <div className="search-retrieval-modes" aria-label="Retrieval mode">
              {(["keyword", "semantic", "hybrid"] as const).map((candidate) => {
                const unavailable =
                  candidate !== "keyword" &&
                  (!semantic.modelInstalled || semantic.indexedPapers === 0);
                return (
                  <button
                    type="button"
                    key={candidate}
                    className={mode === candidate ? "is-active" : ""}
                    disabled={unavailable}
                    aria-pressed={mode === candidate}
                    title={
                      unavailable
                        ? "Install and index the local semantic model from Data Source"
                        : undefined
                    }
                    onClick={() => setMode(candidate)}
                  >
                    {candidate}
                  </button>
                );
              })}
              <small>
                {mode === "keyword"
                  ? "Exact metadata and full-text terms"
                  : mode === "semantic"
                    ? "Meaning-ranked local passages"
                    : "Keyword and semantic ranks fused locally"}
              </small>
            </div>

            <div
              className="search-citation-filters"
              aria-label="Citation filters"
            >
              {citationFilters.map((filter) => (
                <button
                  type="button"
                  key={filter.id}
                  className={citationFilter === filter.id ? "is-active" : ""}
                  aria-pressed={citationFilter === filter.id}
                  onClick={() => setCitationFilter(filter.id)}
                >
                  {filter.label}
                </button>
              ))}
            </div>

            <div className="search-selection-bar">
              <span>Check papers to collect across searches</span>
              <button type="button" onClick={onReviewSelection}>
                Review selection ({selectedIds.length})
                <ArrowRight size={14} />
              </button>
              <span className="sr-only" role="status">
                {selectedIds.length} papers selected
              </span>
            </div>

            {query.trim() === "" && citationFilter === "all" ? (
              <div className="search-empty">
                <span className="eyebrow">TRY A RESEARCH QUERY</span>
                <div>
                  {exampleQueries.map((example) => (
                    <button
                      type="button"
                      key={example}
                      onClick={() => setQuery(example)}
                    >
                      <Search size={14} />
                      {example}
                      <ArrowRight size={14} />
                    </button>
                  ))}
                </div>
                <p>
                  {sourceMode === "demo"
                    ? "Real paper metadata is searchable immediately; downloaded and indexed text is available in the desktop app."
                    : semantic.modelInstalled
                      ? "Keyword, semantic, and hybrid retrieval run locally. No library content leaves this computer."
                      : "Keyword search is ready. Install the optional local model from Data Source to enable semantic and hybrid retrieval."}
                </p>
              </div>
            ) : (
              <div className="search-results">
                <div className="search-results-heading">
                  <span>{total} MATCHES</span>
                  <small>{corpusStatus} corpus / linked to graph</small>
                </div>
                {mode !== "keyword" && semanticCoverage.eligible > 0 && (
                  <div className="search-coverage-note">
                    Semantic coverage {semanticCoverage.indexed} /{" "}
                    {semanticCoverage.eligible} papers
                    {semanticCoverage.indexed < semanticCoverage.eligible
                      ? " - results are partial while indexing continues"
                      : ""}
                  </div>
                )}
                {searchError && (
                  <div className="search-error">{searchError}</div>
                )}
                {results.map((result, index) => (
                  <div
                    key={result.item.id}
                    className={`search-result ${selectedIds.includes(result.item.id) ? "is-selected" : ""}`}
                    onMouseEnter={() => setActiveIndex(index)}
                  >
                    <input
                      type="checkbox"
                      className="result-checkbox"
                      checked={selectedIds.includes(result.item.id)}
                      onChange={() => onTogglePaper(result.item)}
                      aria-label={`Select ${result.item.title}`}
                    />
                    <button
                      type="button"
                      className={`search-result-open ${activeIndex === index ? "is-active" : ""}`}
                      onClick={() => chooseResult(result)}
                      title={`Open ${result.item.title}`}
                    >
                      <span className="result-copy">
                        <strong>{result.item.title}</strong>
                        <small>
                          {result.item.authors[0]?.name ?? "Unknown creator"} /{" "}
                          {result.item.year ?? "No year"} /{" "}
                          {result.item.venue ?? "No venue"}
                        </small>
                        {result.snippet && <em>{result.snippet}</em>}
                        <i>{result.reasons.join(" / ")}</i>
                        {result.passages[0] && (
                          <small className="result-provenance">
                            passage {result.passages[0].chunkId.slice(0, 8)} /
                            cosine {result.passages[0].semanticScore.toFixed(3)}
                          </small>
                        )}
                      </span>
                      <span className="result-score">
                        {result.retrievalModes
                          .map((retrievalMode) =>
                            retrievalMode.slice(0, 3).toUpperCase(),
                          )
                          .join("+")}
                      </span>
                      {activeIndex === index && <CornerDownLeft size={14} />}
                    </button>
                  </div>
                ))}
                {results.length === 0 && (
                  <div className="no-results">
                    No cached metadata matches this query. Try a title, creator,
                    or Zotero tag.
                  </div>
                )}
                {results.length < total && (
                  <button
                    type="button"
                    className="search-load-more"
                    onClick={() => {
                      const generation = searchGeneration.current;
                      void services.search
                        .search(query, 20, results.length, citationFilter, mode)
                        .then((page) => {
                          if (generation !== searchGeneration.current) return;
                          setResults((current) => [
                            ...new Map(
                              [...current, ...page.results].map((result) => [
                                result.item.id,
                                result,
                              ]),
                            ).values(),
                          ]);
                        })
                        .catch((reason: unknown) => {
                          if (generation === searchGeneration.current)
                            setSearchError(
                              reason instanceof Error
                                ? reason.message
                                : String(reason),
                            );
                        });
                    }}
                  >
                    Load next results
                  </button>
                )}
              </div>
            )}

            <footer className="search-footer">
              <button
                type="button"
                disabled={!results.length}
                onClick={() => {
                  onFocusResults(results.map((result) => result.item.id));
                  onClose();
                }}
              >
                Show {results.length} results on graph
              </button>
              <span>
                <kbd>UP</kbd>
                <kbd>DOWN</kbd>
                Navigate
              </span>
              <span>
                <kbd>
                  <CornerDownLeft size={10} />
                </kbd>
                Focus in graph
              </span>
              <span>
                <kbd>
                  <Command size={10} />
                </kbd>
                <kbd>K</kbd>
                Toggle
              </span>
            </footer>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
