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

process.stdout.write(
  "Repository file hygiene passed: required documentation, ignore rules, " +
    "QA commands, and portable tokenizer configuration are present.\n",
);
