import { mkdir, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const productionReleaseEnvironment = "production-release";
const defaultPathApi = { isAbsolute, relative, resolve };

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

export function validatePublisherName(value) {
  assert(
    value === "Graphite",
    "Release publisher must use the neutral Graphite identity.",
  );
  return value;
}

export function createReleaseMetadata(publisher, year) {
  const validatedPublisher = validatePublisherName(publisher);
  assert(
    Number.isInteger(year) && year >= 2026 && year <= 9999,
    "Release copyright year is invalid.",
  );
  return {
    bundle: {
      copyright: `Copyright (c) ${year} ${validatedPublisher}`,
      publisher: validatedPublisher,
    },
  };
}

export function resolveReleaseMetadataPath(
  runnerTemp,
  pathApi = defaultPathApi,
) {
  assert(
    typeof runnerTemp === "string" && pathApi.isAbsolute(runnerTemp),
    "RUNNER_TEMP must be an absolute path.",
  );
  const outputPath = pathApi.resolve(
    runnerTemp,
    "graphite-production-release",
    "tauri.publisher.conf.json",
  );
  const outputRelative = pathApi.relative(
    pathApi.resolve(runnerTemp),
    outputPath,
  );
  assert(
    outputRelative.length > 0 &&
      !outputRelative.startsWith("..") &&
      !pathApi.isAbsolute(outputRelative),
    "Release metadata must remain below RUNNER_TEMP.",
  );
  return outputPath;
}

async function main() {
  assert(
    process.env.CI === "true" && process.env.GITHUB_ACTIONS === "true",
    "Release metadata generation is restricted to GitHub Actions.",
  );
  assert(
    process.env.GRAPHITE_RELEASE_ENVIRONMENT === productionReleaseEnvironment,
    `Release metadata requires the '${productionReleaseEnvironment}' environment.`,
  );
  const outputPath = resolveReleaseMetadataPath(process.env.RUNNER_TEMP);
  const metadata = createReleaseMetadata(
    process.env.GRAPHITE_RELEASE_PUBLISHER ?? "Graphite",
    new Date().getUTCFullYear(),
  );
  await mkdir(dirname(outputPath), { recursive: true });
  await writeFile(outputPath, `${JSON.stringify(metadata, null, 2)}\n`, {
    encoding: "utf8",
    mode: 0o600,
  });
  process.stdout.write(
    "Release publisher overlay written below RUNNER_TEMP.\n",
  );
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) await main();
