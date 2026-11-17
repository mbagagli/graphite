import { useState } from "react";

export function ReaderPageNumber({
  value,
  maximum,
  onChange,
}: {
  value: number;
  maximum: number;
  onChange: (value: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));
  const [previousValue, setPreviousValue] = useState(value);
  if (previousValue !== value) {
    setPreviousValue(value);
    setDraft(String(value));
  }

  const commit = () => {
    const parsed = Number(draft);
    if (!draft.trim() || !Number.isFinite(parsed)) {
      setDraft(String(value));
      return;
    }
    const next = Math.min(maximum, Math.max(1, Math.trunc(parsed)));
    setDraft(String(next));
    if (next !== value) onChange(next);
  };

  return (
    <input
      name="reader-page-number"
      type="number"
      inputMode="numeric"
      min={1}
      max={maximum}
      value={draft}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          event.preventDefault();
          commit();
          event.currentTarget.select();
        } else if (event.key === "Escape") {
          event.stopPropagation();
          setDraft(String(value));
        }
      }}
      aria-label="Page number"
      title="Enter a page number and press Enter"
    />
  );
}
