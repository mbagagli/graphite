import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";

import { version } from "../../package.json";
import { AboutDialog, HelpDialog } from "./HelpDialog";

it("offers the four named help tabs with accessible relationships", () => {
  const markup = renderToStaticMarkup(<HelpDialog onClose={vi.fn()} />);
  for (const title of ["START-UP GUIDE", "HOW-TO", "RAG-SYSTEM", "TIPS"]) {
    expect(markup).toContain(title);
  }
  expect(markup.match(/role="tab"/g)).toHaveLength(4);
  expect(markup).toContain('role="tabpanel"');
  expect(markup).toContain('aria-labelledby="help-tab-0"');
  expect(markup).not.toContain("synthetic");
  expect(markup).toContain("Prepare library for AI");
  expect(markup).toContain("Already downloaded?");
  expect(markup).toContain("real seismology collection");
  expect(markup).toContain("Use Zotero");
});

it("shows the authoritative package version and verified license", () => {
  const markup = renderToStaticMarkup(<AboutDialog onClose={vi.fn()} />);
  expect(markup).toContain(`<dd>${version}</dd>`);
  expect(markup).toContain("<dt>Created by:</dt><dd>Matteo Bagagli</dd>");
  expect(markup).toContain("Windows is currently unavailable");
  expect(markup).toContain("GPL-3.0-only");
  expect(markup).toContain("Support and diagnostics");
  expect(markup).toContain("without warranty");
});
