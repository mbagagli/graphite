import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, expect, it, vi } from "vitest";
import { PanelDivider } from "./PanelDivider";

it("exposes a keyboard-adjustable horizontal separator with bounded sizing", () => {
  const html = renderToStaticMarkup(
    <PanelDivider storageKey="test-split" label="Resize panels" enabled />,
  );
  expect(html).toContain('role="separator"');
  expect(html).toContain('aria-orientation="horizontal"');
  expect(html).toContain('aria-valuemin="20"');
  expect(html).toContain('aria-valuemax="80"');
  expect(html).toContain('aria-valuenow="50"');
  expect(html).toContain('tabindex="0"');
});
it("removes the separator from keyboard navigation when a panel is collapsed", () => {
  const html = renderToStaticMarkup(
    <PanelDivider
      storageKey="test-split"
      label="Resize panels"
      enabled={false}
    />,
  );
  expect(html).toContain('aria-hidden="true"');
  expect(html).not.toContain("tabindex");
  expect(html).not.toContain('role="separator"');
});

afterEach(() => vi.unstubAllGlobals());

it.each([
  [null, 50],
  ["", 50],
  ["invalid", 50],
  ["NaN", 50],
  ["Infinity", 50],
  ["35", 35],
  ["72.5", 73],
  ["10", 20],
  ["90", 80],
])("restores stored %s as a bounded split of %s", (stored, expected) => {
  vi.stubGlobal("window", { localStorage: { getItem: () => stored } });
  const html = renderToStaticMarkup(
    <PanelDivider storageKey="test-split" label="Resize panels" enabled />,
  );
  expect(html).toContain(`aria-valuenow="${expected}"`);
});

it("restores library and reader positions independently on remount", () => {
  const values = new Map([
    ["library", "35"],
    ["reader", "70"],
  ]);
  vi.stubGlobal("window", {
    localStorage: { getItem: (key: string) => values.get(key) ?? null },
  });
  for (const key of ["library", "reader", "library", "reader"]) {
    const html = renderToStaticMarkup(
      <PanelDivider storageKey={key} label="Resize panels" enabled />,
    );
    expect(html).toContain(`aria-valuenow="${values.get(key)}"`);
  }
});

it("remains usable when storage access is denied", () => {
  const warning = vi.spyOn(console, "warn").mockImplementation(() => {});
  vi.stubGlobal("window", {
    get localStorage() {
      throw new Error("Storage disabled");
    },
  });
  try {
    const html = renderToStaticMarkup(
      <PanelDivider storageKey="test-split" label="Resize panels" enabled />,
    );
    expect(html).toContain('aria-valuenow="50"');
    expect(warning).toHaveBeenCalledOnce();
  } finally {
    warning.mockRestore();
  }
});
