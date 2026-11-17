import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "..");
const documentationPaths = [
  "CODE_SIGNING_POLICY.md",
  "LICENSE",
  "PRIVACY.md",
  "README.md",
  "docs/development.md",
  "docs/releasing.md",
  "docs/release/BUILD.md",
  "docs/release/SECURITY.md",
];
const requiredIgnoreRules = [
  "/.cache/",
  "/artifacts/",
  "coverage/",
  "dist/",
  "/dist-site/",
  "/*",
  "AGENTS.md",
  "ROADMAP.md",
  "HANDOFF.md",
  ".agents/",
  ".codex/",
  ".impeccable/",
  "/indexes/",
  "/models/",
  "/runtime-data/",
  "*.sqlite3",
  ".env",
];
const qaScriptPaths = [
  "scripts/performance-qa.mjs",
  "scripts/qa-runtime.mjs",
  "scripts/visual-qa.mjs",
];

for (const path of documentationPaths) {
  await readFile(resolve(repositoryRoot, path));
}

const ignoreRules = new Set(
  (await readFile(resolve(repositoryRoot, ".gitignore"), "utf8"))
    .split(/\r?\n/)
    .map((line) => line.trim()),
);
for (const rule of requiredIgnoreRules) {
  if (!ignoreRules.has(rule)) {
    throw new Error(
      `Required generated/private ignore rule is missing: ${rule}`,
    );
  }
}
for (const path of qaScriptPaths) {
  const source = await readFile(resolve(repositoryRoot, path), "utf8");
  if (source.includes("/snap/bin/") || /["']\/tmp\//.test(source)) {
    throw new Error(`${path} contains a host-specific executable or path.`);
  }
}

const packageMetadata = JSON.parse(
  await readFile(resolve(repositoryRoot, "package.json"), "utf8"),
);
for (const scriptName of [
  "qa:performance",
  "qa:performance:large",
  "qa:visual",
]) {
  if (!packageMetadata.scripts[scriptName]) {
    throw new Error(`package.json is missing the '${scriptName}' command.`);
  }
}
if (!packageMetadata.scripts.dev.includes("--host 127.0.0.1")) {
  throw new Error("The Vite development command must remain loopback-only.");
}

const tauriConfig = JSON.parse(
  await readFile(resolve(repositoryRoot, "src-tauri/tauri.conf.json"), "utf8"),
);
if (tauriConfig.build.devUrl !== "http://127.0.0.1:1420") {
  throw new Error("The Tauri development URL must remain loopback-only.");
}
if (tauriConfig.bundle?.macOS?.minimumSystemVersion !== "11.0") {
  throw new Error(
    "The macOS deployment target must support the native C++ dependency graph.",
  );
}

const cargoManifest = await readFile(
  resolve(repositoryRoot, "src-tauri/Cargo.toml"),
  "utf8",
);
if (
  !/^tokenizers\s*=\s*\{[^\n]*default-features\s*=\s*false[^\n]*features\s*=\s*\["onig"\][^\n]*\}$/m.test(
    cargoManifest,
  )
) {
  throw new Error(
    "Tokenizers must not enable the Windows-incompatible ESAXX C++ feature.",
  );
}

const ciWorkflow = await readFile(
  resolve(repositoryRoot, ".github/workflows/ci.yml"),
  "utf8",
);
if (!/permissions:\s*\n\s+contents: read/.test(ciWorkflow)) {
  throw new Error("CI must retain read-only repository permissions.");
}
for (const forbidden of [
  "pull_request_target",
  "actions/upload-artifact",
  "secrets.",
  "0.0.0.0",
]) {
  if (ciWorkflow.includes(forbidden)) {
    throw new Error(`CI contains forbidden content: ${forbidden}`);
  }
}
for (const required of [
  "MACOSX_DEPLOYMENT_TARGET=11.0",
  'ORT_SKIP_DOWNLOAD: "1"',
]) {
  if (!ciWorkflow.includes(required)) {
    throw new Error(`CI is missing required native configuration: ${required}`);
  }
}
const actionReferences = [
  ...ciWorkflow.matchAll(/uses:\s*([^@\s]+)@([^\s#]+)/g),
];
for (const [, action, revision] of actionReferences) {
  if (!action.startsWith("actions/") || !/^[a-f0-9]{40}$/.test(revision)) {
    throw new Error(
      `CI action '${action}' must be GitHub-authored and pinned to a full SHA.`,
    );
  }
}
if (actionReferences.length === 0) {
  throw new Error("CI does not declare any pinned actions.");
}

process.stdout.write(
  "Repository file hygiene passed: required documentation, ignore rules, QA commands, " +
    "loopback bindings, and CI safety are consistent.\n",
);
