import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
import { LensEditor } from "./LensEditor";

it.each([
  { selection: ["included"], count: 0, disabled: true },
  { selection: ["included", "new"], count: 1, disabled: false },
])("offers only selected papers absent from the Lens: $count", (scenario) => {
  const markup = renderToStaticMarkup(
    <LensEditor
      libraryId="library"
      lens={{
        id: "lens",
        name: "Evidence",
        description: "",
        paperIds: ["included"],
        groups: [],
        saved: true,
        revision: 1,
      }}
      selection={scenario.selection}
      papers={[]}
      onClose={vi.fn()}
      onSaved={vi.fn()}
      onDeleted={vi.fn()}
    />,
  );
  const button = markup.match(
    /<button[^>]*>Add selected papers \(\d+\)<\/button>/,
  )?.[0];
  expect(button).toContain(`(${scenario.count})`);
  expect(button?.includes("disabled")).toBe(scenario.disabled);
  expect(markup).toContain(">Add papers</button>");
  expect(markup).not.toContain("reopen this editor to add them");
  if (scenario.disabled)
    expect(markup).toContain("Your selected papers are already in this Lens.");
});
