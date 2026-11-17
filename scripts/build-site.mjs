import { createHash } from "node:crypto";
import { copyFile, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath, URL } from "node:url";

import { validateManifest } from "./real-demo.mjs";
import { checkPublicDirectory } from "./privacy-check.mjs";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const showcaseImages = ["search.png", "reader.png"];

export function escapeHtml(value) {
  return String(value).replace(
    /[&<>"']/g,
    (character) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#39;",
      })[character],
  );
}

export function siteConfiguration(environment = process.env) {
  const base = environment.SITE_BASE || "/graphite/";
  if (!/^\/(?:[A-Za-z0-9_-]+\/)*$/.test(base)) {
    throw new Error(
      "SITE_BASE must be / or slash-delimited safe path segments.",
    );
  }
  const repository = environment.GITHUB_REPOSITORY || "";
  if (repository && !/^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/.test(repository)) {
    throw new Error("GITHUB_REPOSITORY must have the form owner/repository.");
  }
  if (environment.SITE_PUBLIC === "true" && !repository) {
    throw new Error("A public build requires GITHUB_REPOSITORY.");
  }
  return { base, repository };
}

export function downloadMarkup(repository, version) {
  if (!/^\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?$/.test(version)) {
    throw new Error("The package version is not a release version.");
  }
  const targets = [
    ["Linux Debian", "x86_64 / .deb", `Graphite_${version}_amd64.deb`],
    [
      "Linux AppImage",
      "x86_64 / .AppImage",
      `Graphite_${version}_amd64.AppImage`,
    ],
    ["macOS", "Apple Silicon / .dmg", `Graphite_${version}_aarch64.dmg`],
  ];
  const releaseUrl = repository
    ? `https://github.com/${repository}/releases/download/v${version}`
    : "";
  const entries = targets.map(([label, architecture, file]) => {
    const action = releaseUrl
      ? `<a href="${escapeHtml(`${releaseUrl}/${file}`)}">Download after publication</a>`
      : "<span>Awaiting a public release destination</span>";
    return `<div><strong>${label}</strong><span>${architecture}</span>${action}</div>`;
  });
  const checksums = releaseUrl
    ? `<p><a href="${escapeHtml(`${releaseUrl}/SHA256SUMS.txt`)}">Release checksums</a></p>`
    : "";
  return `<div class="downloads">${entries.join("")}</div>${checksums}`;
}

function safeWebLink(value) {
  const url = new URL(value);
  if (url.protocol !== "https:" || url.username || url.password) {
    throw new Error("Public source links must use HTTPS without credentials.");
  }
  return escapeHtml(url.href);
}

export function attributionMarkup(manifest) {
  validateManifest(manifest);
  const entries = manifest.papers.map((paper) => {
    return (
      `<li><a href="${safeWebLink(paper.sourceUrl)}">${escapeHtml(paper.title)}</a>` +
      `<small>${escapeHtml(paper.authors.join(", "))} (${paper.year}). ` +
      `DOI: ${escapeHtml(paper.doi)}.</small>` +
      `<small>${escapeHtml(paper.attribution)} ` +
      `<a href="${safeWebLink(paper.licenseUrl)}">${escapeHtml(paper.license)}</a> / ` +
      `<a href="${safeWebLink(paper.rightsEvidenceUrl)}">Rights evidence</a></small></li>`
    );
  });
  return `<ol>${entries.join("")}</ol>`;
}

export function screenshotCaption(manifest, manifestBytes, provenance, image) {
  const native = provenance.kind === "native-desktop-capture";
  const browser = provenance.kind === "browser-metadata-capture";
  const year = provenance.screenshotChecks?.year;
  const count = manifest.papers.filter((paper) => paper.year === year).length;
  const verifiedPapers = native
    ? provenance.nativeChecks?.papers
    : provenance.metadataChecks?.papers;
  if (
    (!native && !browser) ||
    provenance.manifestId !== manifest.id ||
    provenance.manifestSha256 !==
      createHash("sha256").update(manifestBytes).digest("hex") ||
    verifiedPapers !== manifest.papers.length ||
    count === 0 ||
    provenance.screenshotChecks?.exactDisplayedPaperCount !== count ||
    provenance.sha256 !== createHash("sha256").update(image).digest("hex")
  ) {
    throw new Error(
      "The real-demo screenshot provenance does not match its image or collection.",
    );
  }
  const context = native
    ? `Graphite Linux candidate, using the verified ${manifest.papers.length}-paper collection offline. `
    : `Graphite browser metadata view of the current ${manifest.papers.length}-paper collection. ` +
      "This image does not demonstrate native PDF import or model execution. ";
  return (
    context +
    `The publication-year popup lists ${count} papers from ${year}. ` +
    "Attribution appears below."
  );
}

export async function buildSite({
  root = repositoryRoot,
  environment = process.env,
} = {}) {
  const { base, repository } = siteConfiguration(environment);
  const metadata = JSON.parse(
    await readFile(resolve(root, "package.json"), "utf8"),
  );
  const output = resolve(root, "dist-site");
  const manifestBytes = await readFile(resolve(root, "demo/manifest.json"));
  const manifest = JSON.parse(manifestBytes);
  const distribution = JSON.parse(
    await readFile(resolve(root, "demo/distribution.json"), "utf8"),
  );
  const demoUrl = new URL(distribution.url);
  if (
    demoUrl.origin !== "https://zenodo.org" ||
    !/^\/records\/\d+\/files\/[^/]+\.zip$/.test(demoUrl.pathname) ||
    demoUrl.username ||
    demoUrl.password ||
    demoUrl.search ||
    demoUrl.hash ||
    distribution.manifestId !== manifest.id ||
    !demoUrl.pathname.endsWith(`/${distribution.fileName}`)
  )
    throw new Error("Pages requires the pinned public Zenodo archive.");
  const demoRecord = `https://doi.org/10.5281/zenodo.${demoUrl.pathname.split("/")[2]}`;
  const attribution = attributionMarkup(manifest);
  const workspaceImage = await readFile(
    resolve(root, "website/assets/workspace.png"),
  );
  const workspaceProvenance = JSON.parse(
    await readFile(
      resolve(root, "website/assets/workspace.provenance.json"),
      "utf8",
    ),
  );
  if (
    workspaceProvenance.appVersion !== metadata.version ||
    workspaceProvenance.manifestSha256 !==
      createHash("sha256").update(manifestBytes).digest("hex") ||
    workspaceProvenance.sha256 !==
      createHash("sha256").update(workspaceImage).digest("hex")
  ) {
    throw new Error("Workspace screenshot provenance does not match.");
  }
  for (const name of showcaseImages) {
    const image = await readFile(resolve(root, "website/assets", name));
    if (
      workspaceProvenance.views?.[name]?.sha256 !==
      createHash("sha256").update(image).digest("hex")
    )
      throw new Error(`Showcase screenshot provenance does not match: ${name}`);
  }
  let preview;
  try {
    const image = await readFile(resolve(root, "website/assets/real-demo.png"));
    const provenance = JSON.parse(
      await readFile(
        resolve(root, "website/assets/real-demo.provenance.json"),
        "utf8",
      ),
    );
    const caption = screenshotCaption(
      manifest,
      manifestBytes,
      provenance,
      image,
    );
    preview =
      '<figure><img class="app-preview" src="__BASE__assets/real-demo.png" ' +
      'width="1440" height="814" alt="Graphite Analytics showing the real seismology ' +
      'collection and the papers in a selected publication year" fetchpriority="high" />' +
      `<figcaption>${caption}</figcaption></figure>`;
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
    preview =
      '<p class="preview-note">The desktop workspace connects graph exploration, ' +
      "search, PDF reading, and Analytics. A verified real-demo screenshot is still " +
      "required before publication.</p>";
    if (environment.SITE_PUBLIC === "true") {
      throw new Error(
        "Public Pages requires the verified real-demo screenshot.",
        { cause: error },
      );
    }
  }
  let html = await readFile(resolve(root, "website/index.html"), "utf8");
  const replacements = {
    __PREVIEW__: preview,
    __DEMO_COUNT__: String(manifest.papers.length),
    __DEMO_STATUS__:
      '<p class="small">The collection manifest records publisher links, ' +
      "rights evidence, and PDF checksums. See the walkthrough for executed checks " +
      "and steps still requiring desktop verification.</p>",
    __ATTRIBUTION__: attribution,
    __DOWNLOADS__: downloadMarkup(repository, metadata.version),
    __SOURCE_LINK__: repository
      ? `<a href="https://github.com/${escapeHtml(repository)}">Source and support</a>`
      : "",
    __VERSION__: metadata.version,
    __DEMO_RECORD__: escapeHtml(demoRecord),
    __DEMO_DOWNLOAD__: safeWebLink(distribution.url),
  };
  for (const [token, value] of Object.entries(replacements))
    html = html.replaceAll(token, value);
  html = html.replaceAll("__BASE__", base);
  if (/__[A-Z_]+__/.test(html))
    throw new Error("Unresolved Pages template tokens.");
  await rm(output, { recursive: true, force: true });
  await mkdir(resolve(output, "assets"), { recursive: true });
  await writeFile(resolve(output, "index.html"), html);
  for (const page of ["manual", "tutorial"]) {
    const template = await readFile(
      resolve(root, `website/${page}.html`),
      "utf8",
    );
    let rendered = template.replaceAll("__BASE__", base);
    for (const [token, value] of Object.entries(replacements))
      rendered = rendered.replaceAll(token, value);
    if (/__[A-Z_]+__/.test(rendered))
      throw new Error("Unresolved guide template tokens.");
    await writeFile(resolve(output, `${page}.html`), rendered);
  }
  const files = [
    ...showcaseImages.map((name) => [
      `website/assets/${name}`,
      `assets/${name}`,
    ]),
    ["website/assets/workspace.png", "assets/workspace.png"],
    [
      "website/assets/workspace.provenance.json",
      "assets/workspace.provenance.json",
    ],
    ["docs/lenses.md", "lenses.txt"],
    ["docs/graph-navigation.md", "graph-navigation.txt"],
    ["docs/help-and-analytics.md", "help-and-analytics.txt"],
    ["docs/pdf-editing.md", "pdf-editing.txt"],
    ["website/style.css", "style.css"],
    ["LICENSE", "LICENSE.txt"],
    ["PRIVACY.md", "privacy.txt"],
    ["docs/release/DEMO.md", "demo-guide.txt"],
    ["docs/library-preparation.md", "library-preparation.txt"],
    ["docs/release/BUILD.md", "installation.txt"],
    ["docs/release/WINDOWS.md", "windows.txt"],
    ["demo/manifest.json", "demo-manifest.json"],
    [
      "node_modules/@fontsource-variable/inter/files/inter-latin-wght-normal.woff2",
      "assets/inter-latin.woff2",
    ],
    [
      "node_modules/@fontsource-variable/inter/LICENSE",
      "assets/Inter-LICENSE.txt",
    ],
  ];
  if (preview.includes("<figure>")) {
    files.push(
      ["website/assets/real-demo.png", "assets/real-demo.png"],
      [
        "website/assets/real-demo.provenance.json",
        "assets/real-demo.provenance.json",
      ],
    );
  }
  for (const [source, destination] of files) {
    await copyFile(resolve(root, source), resolve(output, destination));
  }
  await writeFile(resolve(output, ".nojekyll"), "");
  await checkPublicDirectory(output);
  return { version: metadata.version, base, publicLinks: Boolean(repository) };
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const result = await buildSite();
  process.stdout.write(`Pages preview built: ${JSON.stringify(result)}\n`);
}
