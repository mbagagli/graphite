import { ArrowLeft, Check, Plus, Search } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import type { ResearchItem } from "../../domain/research";
import { MAX_LENS_PAPERS } from "../../domain/researchLens";
import type { SearchPage } from "../../services/contracts";
import { services } from "../../services/runtime";

interface LensPaperPickerProps {
  name: string;
  query: string;
  memberIds: string[];
  originalIds: string[];
  onQueryChange: (query: string) => void;
  onAdd: (paper: ResearchItem) => void;
  onDone: () => void;
}

const PAGE_SIZE = 20;
const SEARCH_DELAY_MS = 150;

export function LensPaperPicker({
  name,
  query,
  memberIds,
  originalIds,
  onQueryChange,
  onAdd,
  onDone,
}: LensPaperPickerProps) {
  const [offset, setOffset] = useState(0);
  const [attempt, setAttempt] = useState(0);
  const [result, setResult] = useState<{
    key: string;
    page: SearchPage | null;
    error: string | null;
  } | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const results = useRef<HTMLDivElement>(null);
  const terms = query.trim();
  const requestKey = JSON.stringify([terms, offset, attempt]);
  const current = result?.key === requestKey ? result : null;
  const loading = Boolean(terms) && !current;
  const page = current?.page;
  const atLimit = memberIds.length >= MAX_LENS_PAPERS;

  useEffect(() => {
    input.current?.focus();
  }, []);

  useEffect(() => {
    if (!terms) return;
    let active = true;
    const timer = window.setTimeout(() => {
      void services.search
        .search(terms, PAGE_SIZE, offset, "all", "keyword")
        .then((page) => {
          if (active) setResult({ key: requestKey, page, error: null });
        })
        .catch((reason: unknown) => {
          if (active)
            setResult({
              key: requestKey,
              page: null,
              error: reason instanceof Error ? reason.message : String(reason),
            });
        });
    }, SEARCH_DELAY_MS);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [terms, offset, requestKey]);

  return (
    <section className="lens-paper-picker" aria-label="Add papers to draft">
      <div className="lens-picker-context">
        <button type="button" onClick={onDone}>
          <ArrowLeft size={14} /> Back to Lens
        </button>
        <span>{name.trim() || "New Lens"}</span>
      </div>
      <p>Add papers to this draft. Nothing changes until you save the Lens.</p>
      <label htmlFor="lens-paper-query">Search this library</label>
      <div className="lens-picker-search">
        <Search size={16} aria-hidden="true" />
        <input
          id="lens-paper-query"
          ref={input}
          value={query}
          placeholder="Search titles, authors, tags, or indexed text"
          autoComplete="off"
          onChange={(event) => {
            setOffset(0);
            onQueryChange(event.target.value);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") event.preventDefault();
          }}
        />
      </div>
      <div className="lens-picker-results" ref={results} aria-busy={loading}>
        {!terms && <p>Search for a paper to add to your Lens.</p>}
        {loading && <p role="status">Searching this library...</p>}
        {current?.error && (
          <div role="alert" className="lens-error">
            <p>Search could not finish. Your Lens draft is unchanged.</p>
            <p>{current.error}</p>
            <button
              type="button"
              onClick={() => setAttempt((value) => value + 1)}
            >
              Retry search
            </button>
          </div>
        )}
        {page && (
          <>
            <p className="lens-picker-matches" role="status">
              {page.results.length
                ? `${offset + 1}-${offset + page.results.length} of ${page.total} matches`
                : "No papers match. Try another title, author, or keyword."}
            </p>
            <ul className="lens-picker-list">
              {page.results.map(({ item }) => {
                const included = memberIds.includes(item.id);
                const label = included
                  ? originalIds.includes(item.id)
                    ? "Already included"
                    : "Added to draft"
                  : "Add";
                return (
                  <li key={item.id} className={included ? "is-included" : ""}>
                    <div>
                      <strong>{item.title}</strong>
                      <small>
                        {item.authors[0]?.name ?? "Unknown creator"} /{" "}
                        {item.year ?? "No year"}
                      </small>
                    </div>
                    <button
                      type="button"
                      disabled={included || atLimit}
                      aria-label={`${label}: ${item.title}`}
                      onClick={() => onAdd(item)}
                    >
                      {included ? <Check size={14} /> : <Plus size={14} />}
                      {label}
                    </button>
                  </li>
                );
              })}
            </ul>
          </>
        )}
      </div>
      {page && (offset > 0 || offset + PAGE_SIZE < page.total) && (
        <nav className="lens-picker-pages" aria-label="Paper search pages">
          <button
            type="button"
            disabled={offset === 0}
            onClick={() => {
              results.current?.scrollTo(0, 0);
              setOffset((value) => Math.max(0, value - PAGE_SIZE));
            }}
          >
            Previous results
          </button>
          <button
            type="button"
            disabled={offset + PAGE_SIZE >= page.total}
            onClick={() => {
              results.current?.scrollTo(0, 0);
              setOffset((value) => value + PAGE_SIZE);
            }}
          >
            Next results
          </button>
        </nav>
      )}
      {atLimit && (
        <p role="status">
          A Lens holds up to {MAX_LENS_PAPERS} papers. Return to the draft to
          remove papers before adding more.
        </p>
      )}
      <footer className="lens-picker-footer">
        <span role="status">
          {memberIds.length} {memberIds.length === 1 ? "paper" : "papers"} in
          draft
        </span>
        <button type="button" onClick={onDone}>
          Done adding
        </button>
      </footer>
    </section>
  );
}
