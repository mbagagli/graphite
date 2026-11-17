import { describe, expect, it } from "vitest";
import { browserDemoServices as services } from "./BrowserDemoServices";
import { demoItems, demoManifest } from "./demoData";

describe("real demo browser adapter", () => {
  it("keeps the full library when focusing a paper without a limit", async () => {
    const seedItemIds = [demoItems[0].id];
    const full = await services.graph.loadProjection({ seedItemIds });
    expect(full.items).toHaveLength(60);
    expect(full.graph.nodes).toHaveLength(60);
    expect(full.graph.hasMore).toBe(false);
    const lens = await services.graph.loadProjection({ seedItemIds, limit: 1 });
    expect(lens.items.map((item) => item.id)).toEqual(seedItemIds);
  });

  it("exposes exactly the verified real papers without invented corpus evidence", async () => {
    const data = await services.workspace.load();
    expect(data.snapshot.items).toHaveLength(60);
    expect(new Set(data.snapshot.items.map((item) => item.doi)).size).toBe(60);
    expect(data.snapshot.items.map((item) => item.title)).toEqual(
      demoManifest.papers.map((item) => item.title),
    );
    expect(data.source.mode).toBe("demo");
    expect(data.snapshot.provenance).toBe("real-demo");
    expect(data.corpus.fullTextCount).toBe(0);
    expect(data.semantic.indexedChunks).toBe(0);
    expect(data.citations.referencesExtracted).toBe(0);
    expect(
      data.graph.edges.every((edge) => edge.relation === "shared-tag"),
    ).toBe(true);
    const connected = new Set(
      data.graph.edges.flatMap((edge) => [edge.source, edge.target]),
    );
    expect(connected).toEqual(new Set(demoItems.map((item) => item.id)));
    for (const edge of data.graph.edges) {
      const source = demoItems.find((item) => item.id === edge.source)!;
      const target = demoItems.find((item) => item.id === edge.target)!;
      expect(source.tags.some((tag) => target.tags.includes(tag))).toBe(true);
    }
  });

  it("queries real years, titles, creators, and DOI metadata", async () => {
    const result = await services.search.search("10.5194/se-12-1683-2021");
    expect(result.results).toHaveLength(1);
    expect(result.results[0].item.title).toContain("Bayesian microseismic");
    expect(result.results[0].passages).toEqual([]);
    expect(result.retrievalMode).toBe("keyword");
    const page = await services.library.query({
      yearFrom: 2021,
      yearTo: 2021,
      sort: "title",
      limit: 2,
      offset: 1,
    });
    expect(page.items).toHaveLength(2);
    expect(page.total).toBe(
      demoItems.filter((item) => item.year === 2021).length,
    );
    expect(page.items.every((item) => item.year === 2021)).toBe(true);
    expect(
      (
        await services.search.search(
          "microseismic",
          20,
          0,
          "references-indexed",
        )
      ).results,
    ).toEqual([]);
    expect(
      (await services.search.search("microseismic", 20, 0, "all", "semantic"))
        .results,
    ).toEqual([]);
  });

  it("offers no synthetic documents or generated answers", async () => {
    const document = await services.document.resolve(
      demoItems[0].attachments[0].documentId,
    );
    expect(document.status).toBe("unavailable");
    expect(document.url).toBeNull();
    expect(document.message).toContain("desktop app");
    expect((await services.agent.environment()).modelProfiles).toEqual([]);
    await expect(services.semantic.installModel()).rejects.toThrow(
      "desktop app",
    );
    expect(
      (await services.semantic.similar([demoItems[0].id])).results,
    ).toEqual([]);
  });
});

it("uses the root and theme collection contract for filtering", async () => {
  const { snapshot } = await services.workspace.load();
  const root = snapshot.collections.find((c) => c.parentId === null)!;
  expect(root.name).toBe("Machine learning in seismology");
  const themes = [
    ...new Set(demoManifest.papers.flatMap((p) => p.themes)),
  ].sort();
  expect(
    snapshot.collections
      .filter((c) => c.parentId === root.id)
      .map((c) => c.name),
  ).toEqual(themes);
  for (const collection of snapshot.collections) {
    const expected = demoManifest.papers.filter(
      (p) => collection.id === root.id || p.themes.includes(collection.name),
    );
    const page = await services.library.query({
      collectionId: collection.id,
      limit: 100,
    });
    expect(page.items.map((p) => p.id).sort()).toEqual(
      expected.map((p) => p.id).sort(),
    );
    const projection = await services.graph.loadProjection({
      collectionId: collection.id,
      seedItemIds: [],
    });
    expect(projection.items.map((p) => p.id).sort()).toEqual(
      expected.map((p) => p.id).sort(),
    );
  }
});

it.skipIf(!process.env.GRAPHITE_DEMO_WORKSPACE)(
  "matches collections and memberships from a real native corpus export",
  async () => {
    const { readFile } = await import("node:fs/promises");
    const native = JSON.parse(
      await readFile(process.env.GRAPHITE_DEMO_WORKSPACE!, "utf8"),
    ).snapshot;
    const browser = (await services.workspace.load()).snapshot;
    const normalize = (snapshot: typeof browser) =>
      snapshot.collections
        .map((collection) => ({
          name: collection.name,
          parent:
            snapshot.collections.find((c) => c.id === collection.parentId)
              ?.name ?? null,
          papers: snapshot.items
            .filter((p) => p.collectionIds.includes(collection.id))
            .map((p) => p.doi)
            .sort(),
        }))
        .sort((a, b) => a.name.localeCompare(b.name, "en"));
    expect(normalize(native)).toEqual(normalize(browser));
  },
);
