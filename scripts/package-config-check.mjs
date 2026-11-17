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
  packageMetadata.scripts["desktop:dev"] === "node scripts/desktop-dev.mjs",
  "desktop:dev must use the automatic native backend launcher.",
);
assert(
  packageMetadata.scripts["package:installed-smoke"] ===
    "node scripts/installed-package-smoke.mjs",
  "Installed package validation must use the guarded repository script.",
);
assert(
  packageMetadata.scripts["package:release-candidate"] ===
    "node scripts/release-candidate.mjs",
  "Release-candidate assembly must use the repository verification script.",
);
assert(
  packageMetadata.scripts["release:metadata"] ===
    "node scripts/release-metadata.mjs",
  "Production publisher metadata must use the guarded repository script.",
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
const cargoVersion = cargoManifest.match(/^version\s*=\s*"([^"]+)"$/m)?.[1];
assert(
  cargoVersion === packageMetadata.version,
  "Cargo.toml and package.json must declare the same application version.",
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

const workflow = (
  await readFile(
    resolve(repositoryRoot, ".github/workflows/package-smoke.yml"),
    "utf8",
  )
).replaceAll("\r\n", "\n");
assert(
  /^on:\n {2}workflow_dispatch:\s*$/m.test(workflow),
  "Package smoke must be owner-triggered with workflow_dispatch.",
);
assert(
  /permissions:\s*\n\s+contents: read/.test(workflow),
  "Package smoke must retain read-only repository permissions.",
);
assert(
  !/^\s+[a-z-]+:\s+write\s*$/m.test(workflow),
  "Package smoke must not request a write permission.",
);

for (const forbidden of [
  "pull_request:",
  "pull_request_target:",
  "push:",
  "schedule:",
  "0.0.0.0",
  "secrets.",
  "softprops/action-gh-release",
  "tauri-apps/tauri-action",
]) {
  assert(
    !workflow.includes(forbidden),
    `Package smoke contains forbidden content: ${forbidden}`,
  );
}

for (const required of [
  "platform: linux-x64",
  "runner: ubuntu-24.04",
  "platform: macos-arm64",
  "runner: macos-15",
  "npm run package:build",
  "npm run cuda:prepare",
  "rustup show active-toolchain",
  'test "$(uname -m)" = arm64',
  "xcrun --find clang",
  "id: inspect_packages",
  "if: ${{ !cancelled() && steps.inspect_packages.outcome == 'success' }}",
  "MACOSX_DEPLOYMENT_TARGET=11.0",
  "npm run package:installed-smoke -- ${{ matrix.platform }}",
  "Assemble unsigned release candidate",
  "npm run package:release-candidate",
  "artifacts/package-smoke/${{ matrix.platform }}/",
  "artifacts/package-downloads/",
  "artifacts/release-candidate/",
  "graphite-${{ steps.version.outputs.value }}-unsigned-release-candidate",
  "retention-days: 7",
  "include-hidden-files: false",
]) {
  assert(
    workflow.includes(required),
    `Package smoke is missing required configuration: ${required}`,
  );
}

assert(
  workflow.indexOf("id: version") < workflow.indexOf("id: inspect_packages"),
  "Record the version before inspection so failed runtime checks still " +
    "retain correctly named build artifacts.",
);

for (const forbiddenTarget of [
  "windows-x64",
  "macos-intel",
  "macos-15-intel",
  "universal",
])
  assert(
    !workflow.toLowerCase().includes(forbiddenTarget),
    `Package smoke must not claim unsupported target '${forbiddenTarget}'.`,
  );

const actionReferences = [...workflow.matchAll(/uses:\s*([^@\s]+)@([^\s#]+)/g)];
assert(
  actionReferences.length === 10,
  "Package smoke must use ten pinned actions.",
);
for (const [, action, revision] of actionReferences) {
  assert(
    action.startsWith("actions/") && /^[a-f0-9]{40}$/.test(revision),
    `Action '${action}' must be GitHub-authored and pinned to a full SHA.`,
  );
}

const uploadReferences = actionReferences.filter(
  ([, action]) => action === "actions/upload-artifact",
);
assert(
  uploadReferences.length === 3,
  "Package smoke must have only platform, candidate, and optional demo uploads.",
);
const downloadReferences = actionReferences.filter(
  ([, action]) => action === "actions/download-artifact",
);
assert(
  downloadReferences.length === 1,
  "Package smoke must have one native-package download for candidate assembly.",
);
const uploadPaths = [
  ...workflow.matchAll(/^[ \t]+path:[ \t]*([^\r\n]+?)[ \t]*$/gm),
].map((match) => match[1]);
assert(
  arraysEqual(uploadPaths, [
    "artifacts/package-smoke/${{ matrix.platform }}/",
    "artifacts/package-downloads/",
    "artifacts/release-candidate/",
    "artifacts/demo-archive/",
  ]),
  "Package smoke paths must remain limited to reviewed package and demo staging.",
);

const installedSmokeSource = await readFile(
  resolve(repositoryRoot, "scripts/installed-package-smoke.mjs"),
  "utf8",
);
for (const required of [
  'process.env.CI === "true" && process.env.GITHUB_ACTIONS === "true"',
  "Refusing installed smoke because production state already exists",
  "Production startup modified development state",
  "Installed application state was not preserved",
  "Installed startup downloaded an unexpected model asset",
]) {
  assert(
    installedSmokeSource.includes(required),
    `Installed package smoke is missing its safety guard: ${required}`,
  );
}
assert(
  !installedSmokeSource.includes("0.0.0.0") &&
    !installedSmokeSource.includes("secrets."),
  "Installed package smoke must not expose a service or consume secrets.",
);

const releaseCandidateSource = await readFile(
  resolve(repositoryRoot, "scripts/release-candidate.mjs"),
  "utf8",
);
for (const required of [
  'const productionIdentity = "org.graphite.research"',
  "publicRelease: false",
  "signed: false",
  "package checksum failed",
]) {
  assert(
    releaseCandidateSource.includes(required),
    `Release-candidate assembly is missing its safety guard: ${required}`,
  );
}

const releaseMetadataSource = await readFile(
  resolve(repositoryRoot, "scripts/release-metadata.mjs"),
  "utf8",
);
for (const required of [
  'process.env.CI === "true" && process.env.GITHUB_ACTIONS === "true"',
  'const productionReleaseEnvironment = "production-release"',
  "Release publisher must use the neutral Graphite identity",
  "Release metadata must remain below RUNNER_TEMP",
]) {
  assert(
    releaseMetadataSource.includes(required),
    `Release metadata generation is missing its safety guard: ${required}`,
  );
}
assert(
  !releaseMetadataSource.includes("secrets.") &&
    !releaseMetadataSource.includes("0.0.0.0"),
  "Release metadata generation must not consume secrets or expose a service.",
);

const privacyPolicy = await readFile(
  resolve(repositoryRoot, "PRIVACY.md"),
  "utf8",
);
assert(
  privacyPolicy.includes("has no telemetry") &&
    privacyPolicy.includes("unless specifically requested by the user"),
  "The privacy policy must retain the local-first and explicit-network guarantees.",
);
const codeSigningPolicy = await readFile(
  resolve(repositoryRoot, "CODE_SIGNING_POLICY.md"),
  "utf8",
);
for (const required of [
  "# Code signing policy",
  "production-release",
  "Developer ID Application",
]) {
  assert(
    codeSigningPolicy.includes(required),
    `The code-signing policy is missing required preparation: ${required}`,
  );
}

process.stdout.write(
  "Packaging configuration passed: identities, targets, resources, icons, " +
    "and the owner-triggered workflow are consistent.\n",
);
