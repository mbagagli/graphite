import { readFile } from "node:fs/promises";
import { URL } from "node:url";
import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import {
  attributionMarkup,
  downloadMarkup,
  escapeHtml,
  siteConfiguration,
  screenshotCaption,
} from "./build-site.mjs";

const realManifest = JSON.parse(
  await readFile(new URL("../demo/manifest.json", import.meta.url), "utf8"),
);
const paperCount = realManifest.papers.length;
const selectedYear = 2020;
const yearCount = realManifest.papers.filter(
  (paper) => paper.year === selectedYear,
).length;

function collection() {
  return globalThis.structuredClone(realManifest);
}

describe("Pages build contract", () => {
  it("supports root and project base paths without traversal or remote assets", () => {
    expect(siteConfiguration({}).base).toBe("/graphite/");
    expect(siteConfiguration({ SITE_BASE: "/" }).base).toBe("/");
    expect(siteConfiguration({ SITE_BASE: "/research/graphite/" }).base).toBe(
      "/research/graphite/",
    );
    for (const base of ["//host/", "/../", "/a%2fb/", "graphite", "/x?y/"]) {
      expect(() => siteConfiguration({ SITE_BASE: base })).toThrow();
    }
  });
  it("requires an explicit destination for public builds", () => {
    expect(() => siteConfiguration({ SITE_PUBLIC: "true" })).toThrow();
    for (const repository of [
      "name",
      "https://github.com/org/repo",
      'org/repo"',
    ]) {
      expect(() =>
        siteConfiguration({ GITHUB_REPOSITORY: repository }),
      ).toThrow();
    }
  });
  it("derives all installer links and checksums from the same version", () => {
    const html = downloadMarkup("research/graphite", "0.2.3-rc.1");
    expect(html.match(/releases\/download\/v0\.2\.3-rc\.1/g)).toHaveLength(4);
    expect(html).toContain("Graphite_0.2.3-rc.1_amd64.deb");
    expect(html).toContain("Graphite_0.2.3-rc.1_amd64.AppImage");
    expect(html).toContain("Graphite_0.2.3-rc.1_aarch64.dmg");
    expect(downloadMarkup("", "0.1.0")).not.toContain("href=");
    expect(() => downloadMarkup("research/graphite", "../bad")).toThrow();
  });
  it("escapes publisher metadata and rejects unsafe links and incomplete collections", () => {
    expect(escapeHtml('<script>"&')).toBe("&lt;script&gt;&quot;&amp;");
    const manifest = collection();
    manifest.papers[0].title = "Title <unsafe>";
    expect(attributionMarkup(manifest)).toContain("Title &lt;unsafe&gt;");
    expect(attributionMarkup(manifest).match(/<li>/g)).toHaveLength(paperCount);
    manifest.papers[0].sourceUrl = "javascript:alert(1)";
    expect(() => attributionMarkup(manifest)).toThrow();
    manifest.papers.pop();
    expect(() => attributionMarkup(manifest)).toThrow();
  });
  it("rejects duplicate identities and absent PDF checksums", () => {
    const manifest = collection();
    manifest.papers[1].doi = manifest.papers[0].doi;
    expect(() => attributionMarkup(manifest)).toThrow();
    manifest.papers[1].doi = "10.1234/distinct";
    manifest.papers[0].sha256 = "pending";
    expect(() => attributionMarkup(manifest)).toThrow();
  });
});

it("builds only allowlisted public files at the project base path", async () => {
  const { mkdtemp, mkdir, writeFile, readFile, readdir, rm } =
    await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const { resolve } = await import("node:path");
  const { buildSite } = await import("./build-site.mjs");
  const root = await mkdtemp(resolve(tmpdir(), "graphite-pages-test-"));
  try {
    const files = {
      "package.json": JSON.stringify({ version: "0.1.0" }),
      "demo/manifest.json": JSON.stringify(collection()),
      "demo/distribution.json": JSON.stringify({
        manifestId: collection().id,
        fileName: "demo.zip",
        url: "https://zenodo.org/records/24282715/files/demo.zip",
      }),
      "website/index.html":
        '<a href="__BASE__">Home</a>__PREVIEW____ATTRIBUTION____DOWNLOADS____VERSION____SOURCE_LINK____DEMO_STATUS__<a href="__DEMO_RECORD__">Demo</a>Captured in Graphite __WORKSPACE_VERSION__',
      "website/manual.html":
        '<a href="__BASE__tutorial.html">Tutorial</a><a href="__DEMO_DOWNLOAD__">ZIP</a>',
      "website/tutorial.html": '<a href="__BASE__manual.html">Manual</a>',
      "website/assets/workspace.png": "graph screenshot",
      "website/assets/search.png": "search screenshot",
      "website/assets/reader.png": "reader screenshot",
      "website/assets/workspace.provenance.json": JSON.stringify({
        appVersion: "0.1.0",
        views: Object.fromEntries(
          ["search", "reader"].map((name) => [
            `${name}.png`,
            {
              sha256: createHash("sha256")
                .update(`${name} screenshot`)
                .digest("hex"),
            },
          ]),
        ),
        manifestSha256: createHash("sha256")
          .update(JSON.stringify(collection()))
          .digest("hex"),
        sha256: createHash("sha256").update("graph screenshot").digest("hex"),
      }),
      "docs/lenses.md": "Saved Lens guide",
      "docs/graph-navigation.md": "Graph guide",
      "docs/help-and-analytics.md": "Analytics guide",
      "docs/pdf-editing.md": "PDF guide",
      "website/style.css": "body {color: white}",
      LICENSE: "License fixture",
      "PRIVACY.md": "Privacy fixture",
      "docs/release/DEMO.md": "Demo guide",
      "docs/library-preparation.md": "Prepare library for AI: demo and Zotero",
      "docs/release/BUILD.md": "Installation guide",
      "docs/release/WINDOWS.md": "Windows assessment",
      "node_modules/@fontsource-variable/inter/files/inter-latin-wght-normal.woff2":
        "font fixture",
      "node_modules/@fontsource-variable/inter/LICENSE": "Font license",
      "website/private-note.txt": "Never publish this file",
    };
    for (const [name, contents] of Object.entries(files)) {
      const path = resolve(root, name);
      await mkdir(resolve(path, ".."), { recursive: true });
      await writeFile(path, contents);
    }
    await buildSite({ root, environment: {} });
    const html = await readFile(resolve(root, "dist-site/index.html"), "utf8");
    expect(html).toContain('href="/graphite/"');
    expect(
      await readFile(resolve(root, "dist-site/manual.html"), "utf8"),
    ).toContain('href="/graphite/tutorial.html"');
    expect(
      await readFile(resolve(root, "dist-site/tutorial.html"), "utf8"),
    ).toContain('href="/graphite/manual.html"');
    expect(html).not.toMatch(/__[A-Z_]+__/);
    expect(html).toContain("https://doi.org/10.5281/zenodo.24282715");
    expect(
      await readFile(resolve(root, "dist-site/manual.html"), "utf8"),
    ).toContain("https://zenodo.org/records/24282715/files/demo.zip");
    expect(html).toContain("screenshot is still required");
    expect(
      await readFile(
        resolve(root, "dist-site/library-preparation.txt"),
        "utf8",
      ),
    ).toBe("Prepare library for AI: demo and Zotero");
    expect(await readdir(resolve(root, "dist-site"))).not.toContain(
      "private-note.txt",
    );
    await expect(
      buildSite({
        root,
        environment: {
          SITE_PUBLIC: "true",
          GITHUB_REPOSITORY: "research/graphite",
        },
      }),
    ).rejects.toThrow("screenshot");
    const image = "Verified screenshot fixture";
    const imagePath = resolve(root, "website/assets/real-demo.png");
    const provenancePath = resolve(
      root,
      "website/assets/real-demo.provenance.json",
    );
    await mkdir(resolve(imagePath, ".."), { recursive: true });
    await writeFile(imagePath, image);
    await writeFile(
      provenancePath,
      JSON.stringify({
        manifestId: collection().id,
        kind: "native-desktop-capture",
        nativeChecks: { papers: paperCount },
        manifestSha256: createHash("sha256")
          .update(JSON.stringify(collection()))
          .digest("hex"),
        screenshotChecks: {
          year: selectedYear,
          exactDisplayedPaperCount: yearCount,
        },
        sha256: createHash("sha256").update(image).digest("hex"),
      }),
    );
    const publicEnvironment = {
      SITE_PUBLIC: "true",
      GITHUB_REPOSITORY: "research/graphite",
    };
    await buildSite({ root, environment: publicEnvironment });
    const published = await readFile(
      resolve(root, "dist-site/index.html"),
      "utf8",
    );
    expect(published).toContain("Graphite Linux candidate");
    expect(
      await readFile(resolve(root, "dist-site/assets/reader.png"), "utf8"),
    ).toBe("reader screenshot");
    await writeFile(
      resolve(root, "package.json"),
      JSON.stringify({ version: "0.1.1" }),
    );
    await buildSite({ root, environment: publicEnvironment });
    const bumped = await readFile(
      resolve(root, "dist-site/index.html"),
      "utf8",
    );
    expect(bumped).toContain("Graphite_0.1.1_aarch64.dmg");
    expect(bumped).toContain("Captured in Graphite 0.1.0");
    expect(
      await readFile(
        resolve(root, "dist-site/assets/workspace.provenance.json"),
        "utf8",
      ),
    ).toBe(files["website/assets/workspace.provenance.json"]);
    const workspaceProvenancePath = resolve(
      root,
      "website/assets/workspace.provenance.json",
    );
    for (const appVersion of [undefined, "", "invalid", "<script>"]) {
      const provenance = JSON.parse(
        files["website/assets/workspace.provenance.json"],
      );
      provenance.appVersion = appVersion;
      await writeFile(workspaceProvenancePath, JSON.stringify(provenance));
      await expect(
        buildSite({ root, environment: publicEnvironment }),
      ).rejects.toThrow("Workspace screenshot provenance");
    }
    await writeFile(
      workspaceProvenancePath,
      files["website/assets/workspace.provenance.json"],
    );
    await writeFile(
      resolve(root, "package.json"),
      JSON.stringify({ version: "0.1.0" }),
    );
    await writeFile(
      resolve(root, "website/assets/reader.png"),
      "unverified reader capture",
    );
    await expect(
      buildSite({ root, environment: publicEnvironment }),
    ).rejects.toThrow("Showcase screenshot provenance");
    await writeFile(
      resolve(root, "website/assets/reader.png"),
      "reader screenshot",
    );
    expect(
      await readFile(resolve(root, "dist-site/assets/real-demo.png"), "utf8"),
    ).toBe(image);
    await buildSite({ root, environment: { SITE_BASE: "/" } });
    expect(
      await readFile(resolve(root, "dist-site/manual.html"), "utf8"),
    ).toContain('href="/tutorial.html"');
    const workspacePath = resolve(root, "website/assets/workspace.png");
    await writeFile(workspacePath, "changed graph screenshot");
    await expect(
      buildSite({ root, environment: publicEnvironment }),
    ).rejects.toThrow("Workspace screenshot provenance");
    await writeFile(workspacePath, "graph screenshot");
    await writeFile(imagePath, "Changed screenshot without new evidence");
    await expect(
      buildSite({ root, environment: publicEnvironment }),
    ).rejects.toThrow("provenance");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

it("ties preview year counts and metadata-only captions to the current manifest", () => {
  const manifest = collection();
  const bytes = JSON.stringify(manifest);
  const image = "browser capture";
  const provenance = {
    kind: "browser-metadata-capture",
    manifestId: manifest.id,
    manifestSha256: createHash("sha256").update(bytes).digest("hex"),
    metadataChecks: { papers: paperCount },
    screenshotChecks: {
      year: selectedYear,
      exactDisplayedPaperCount: yearCount,
    },
    sha256: createHash("sha256").update(image).digest("hex"),
  };
  expect(screenshotCaption(manifest, bytes, provenance, image)).toContain(
    "does not demonstrate native PDF import",
  );
  provenance.screenshotChecks.exactDisplayedPaperCount = yearCount + 1;
  expect(() => screenshotCaption(manifest, bytes, provenance, image)).toThrow(
    "provenance",
  );
  provenance.screenshotChecks.exactDisplayedPaperCount = yearCount;
  expect(() =>
    screenshotCaption(manifest, bytes + " ", provenance, image),
  ).toThrow("provenance");
});
