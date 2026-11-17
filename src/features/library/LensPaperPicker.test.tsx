import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";

import { MAX_LENS_PAPERS } from "../../domain/researchLens";
import { LensPaperPicker } from "./LensPaperPicker";

it.each([0, MAX_LENS_PAPERS])(
  "explains empty search and the draft limit at %i papers",
  (count) => {
    const markup = renderToStaticMarkup(
      <LensPaperPicker
        name="Phase picking"
        query=""
        memberIds={Array.from({ length: count }, (_, index) => String(index))}
        originalIds={[]}
        onQueryChange={vi.fn()}
        onAdd={vi.fn()}
        onDone={vi.fn()}
      />,
    );
    expect(markup).toContain("Nothing changes until you save the Lens.");
    expect(markup).toContain('for="lens-paper-query"');
    expect(markup).toContain("Search for a paper to add to your Lens.");
    expect(markup).not.toContain('type="submit"');
    expect(markup.includes("Return to the draft to remove papers")).toBe(
      count === MAX_LENS_PAPERS,
    );
  },
);
