import { access, readFile } from "node:fs/promises";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import { setVersion } from "./set-version.mjs";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "..");
const tauriDirectory = resolve(repositoryRoot, "src-tauri");

const productionIdentity = "org.graphite.research";
const developmentIdentity = "org.graphite.research.dev";
const expectedTargets = new Map([
  ["tauri.linux.conf.json", ["deb", "appimage"]],
  ["tauri.windows.conf.json", ["nsis"]],
  ["tauri.macos.conf.json", ["dmg"]],
]);

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function readJson(path) {
  return JSON.parse(await readFile(path, "utf8"));
}

function arraysEqual(left, right) {
  return (
    Array.isArray(left) &&
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

const packageMetadata = await readJson(resolve(repositoryRoot, "package.json"));
const licensePath = resolve(repositoryRoot, "LICENSE");
const licenseText = await readFile(licensePath, "utf8");
const normalizedLicenseText = licenseText.replace(/\r\n?/g, "\n");
const productionConfig = await readJson(
  resolve(tauriDirectory, "tauri.conf.json"),
);
const developmentConfig = await readJson(
  resolve(tauriDirectory, "tauri.dev.conf.json"),
);

assert(
  productionConfig.identifier === productionIdentity,
  `Production packages must use '${productionIdentity}'.`,
);
assert(
  developmentConfig.identifier === developmentIdentity,
  `Desktop development must use '${developmentIdentity}'.`,
);
assert(
  productionConfig.identifier !== developmentConfig.identifier,
  "Production and development identities must remain distinct.",
);
assert(
  packageMetadata.license === "GPL-3.0-only",
  "package.json must declare GPL-3.0-only.",
);
assert(
  packageMetadata.author === undefined,
  "Package metadata must omit personal authorship.",
);
assert(
  normalizedLicenseText.includes("GNU GENERAL PUBLIC LICENSE") &&
    normalizedLicenseText.includes("Version 3, 29 June 2007") &&
    normalizedLicenseText.includes("END OF TERMS AND CONDITIONS"),
  "LICENSE must contain the complete GNU GPL version 3 text.",
);
await setVersion(repositoryRoot, undefined, true);
assert(
  packageMetadata.scripts.dev.includes("--host 127.0.0.1"),
  "The Vite development server must remain loopback-only.",
);
assert(
  productionConfig.build?.devUrl === "http://127.0.0.1:1420",
  "The Tauri development URL must remain loopback-only.",
);
assert(
  productionConfig.version === "../package.json",
  "Production package versions must use package.json as their source.",
);
assert(
  /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(packageMetadata.version),
  "package.json must contain a valid semantic version for Tauri packages.",
);
const cargoManifest = await readFile(
  resolve(tauriDirectory, "Cargo.toml"),
  "utf8",
);
assert(
  !/^authors\s*=/m.test(cargoManifest),
  "Rust metadata must omit personal authorship.",
);
assert(
  /^license\s*=\s*"GPL-3.0-only"$/m.test(cargoManifest),
  "Cargo.toml must declare GPL-3.0-only.",
);

const bundle = productionConfig.bundle;
assert(bundle?.active === true, "Production Tauri bundling must be active.");
assert(
  bundle.createUpdaterArtifacts === false,
  "Unsigned packaging must not create updater artifacts.",
);
assert(
  bundle.useLocalToolsDir === true,
  "Tauri bundler tools must stay in the ignored repository build cache.",
);
const licenseResources = {
  "licenses/llama-LICENSE.txt": "licenses/llama-LICENSE.txt",
  "../LICENSE": "licenses/Graphite-LICENSE.txt",
  "../node_modules/@embedpdf/pdfium/LICENSE": "licenses/EmbedPDF-LICENSE.txt",
  "../node_modules/@embedpdf/pdfium/LICENSE.pdfium":
    "licenses/PDFium-LICENSE.txt",
  "../src/assets/fonts/NotoSans-LICENSE.txt": "licenses/NotoSans-LICENSE.txt",
};
assert(
  bundle.resources !== null &&
    typeof bundle.resources === "object" &&
    !Array.isArray(bundle.resources) &&
    Object.keys(bundle.resources).length ===
      Object.keys(licenseResources).length &&
    Object.entries(licenseResources).every(
      ([source, destination]) => bundle.resources[source] === destination,
    ),
  "Production packages must include exactly the reviewed project and PDF editing licenses.",
);
for (const source of Object.keys(licenseResources)) {
  assert(
    (await readFile(resolve(repositoryRoot, "src-tauri", source), "utf8"))
      .length > 100,
    `The packaged license is missing or empty: ${source}`,
  );
}
assert(
  bundle.license === "GPL-3.0-only",
  "Tauri bundles must declare GPL-3.0-only.",
);
assert(
  bundle.publisher === "Graphite" &&
    bundle.copyright === "Copyright (c) 2026 Graphite",
  "Tauri bundles must use the neutral project identity.",
);
assert(
  bundle.licenseFile === undefined,
  "The GPL v3 license must be a packaged resource, not an interactive installer agreement.",
);
assert(
  bundle.macOS?.minimumSystemVersion === "11.0",
  "The macOS package deployment floor must remain 11.0.",
);
assert(
  Array.isArray(bundle.icon) && bundle.icon.length > 0,
  "Production packages must declare canonical application icons.",
);

for (const icon of bundle.icon) {
  assert(
    !isAbsolute(icon),
    `Bundle icon '${icon}' must be repository-relative.`,
  );
  const resolvedIcon = resolve(tauriDirectory, icon);
  assert(
    !relative(resolve(tauriDirectory, "icons"), resolvedIcon).startsWith(".."),
    `Bundle icon '${icon}' must remain inside src-tauri/icons.`,
  );
  await access(resolvedIcon);
}

for (const [filename, targets] of expectedTargets) {
  const platformConfig = await readJson(resolve(tauriDirectory, filename));
  assert(
    arraysEqual(platformConfig.bundle?.targets, targets),
    `${filename} must declare only: ${targets.join(", ")}.`,
  );
  if (filename === "tauri.linux.conf.json") {
    assert(
      platformConfig.bundle?.linux?.deb?.depends?.includes("libgomp1"),
      "The Linux DEB must declare the native OpenMP runtime libgomp1.",
    );
  }
  if (filename === "tauri.macos.conf.json") {
    assert(
      platformConfig.bundle?.macOS?.signingIdentity === "-",
      "macOS releases must use the local ad-hoc signature without an Apple account.",
    );
  }
}

process.stdout.write(
  "Packaging configuration passed: versions, identities, targets, " +
    "resources, licenses, and icons are consistent.\n",
);
