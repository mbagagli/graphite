import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
import type { LibraryPreparationController } from "./useLibraryPreparation";
import { LibraryPreparationControl } from "./LibraryPreparationControl";

const controller: LibraryPreparationController = {
  progress: null,
  busy: false,
  archive: null,
  archiveError: null,
  demoCached: false,
  start: vi.fn(),
  cancel: vi.fn(),
};

it("offers one primary entry and explains download size before starting", () => {
  const html = renderToStaticMarkup(
    <LibraryPreparationControl
      preparation={controller}
      demo
      desktop
      disabled={false}
    />,
  );
  expect(html).toContain("Prepare library for AI");
  expect(html).toContain("412 MB");
  expect(html).not.toContain("Import offline PDFs");
  expect(html).not.toContain("Choose ZIP");
  expect(html.match(/<button /g)).toHaveLength(1);
});

it("keeps truthful progress and cancellation when the panel is reopened", () => {
  const html = renderToStaticMarkup(
    <LibraryPreparationControl
      preparation={{
        ...controller,
        busy: true,
        progress: {
          libraryId: "library",
          status: "running",
          stage: "text",
          detail: "Preparing document text",
          completed: 5,
          total: 60,
          unit: "documents",
          issues: [],
        },
      }}
      demo
      desktop
      disabled={false}
    />,
  );
  expect(html).toContain('value="5"');
  expect(html).toContain('max="60"');
  expect(html).toContain("5 / 60 documents");
  expect(html).toContain("Cancel");
});

it("shows indeterminate work and accessible partial coverage separately", () => {
  const progress = {
    libraryId: "library",
    status: "running" as const,
    stage: "citations" as const,
    detail: "Preparing connections",
    completed: 0,
    total: null,
    unit: "documents" as const,
    issues: [{ itemId: "paper", title: "A paper", message: "Needs OCR" }],
  };
  const html = renderToStaticMarkup(
    <LibraryPreparationControl
      preparation={{ ...controller, busy: true, progress }}
      demo={false}
      desktop
      disabled={false}
    />,
  );
  expect(html).not.toContain('value="0"');
  expect(html).toContain("1 item needs attention");
  expect(html).toContain("Needs OCR");
});
