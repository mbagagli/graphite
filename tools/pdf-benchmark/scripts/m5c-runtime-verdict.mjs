#!/usr/bin/env node

import { readdir, readFile } from "node:fs/promises";
import { resolve } from "node:path";
import process from "node:process";

const options = parseArguments(process.argv.slice(2));
await verifyRuntime(options);

function parseArguments(argumentsList) {
  const values = new Map();
  for (let index = 0; index < argumentsList.length; index += 2) {
    const name = argumentsList[index];
    const value = argumentsList[index + 1];
    if (!name?.startsWith("--") || value === undefined) {
      throw new Error(usage());
    }
    values.set(name.slice(2), value);
  }
  const run = values.get("run");
  const candidate = values.get("candidate");
  const profile = values.get("profile");
  if (!run || !candidate || !profile) {
    throw new Error(usage());
  }
  return { run: resolve(run), candidate, profile };
}

function usage() {
  return [
    "Verify an isolated candidate role produced usable synthetic output.",
    "",
    "The command prevents a platform workflow from passing merely because an",
    "adapter serialized explicit unavailable or failed outcomes.",
    "",
    "Processing flow:",
    "  1. Read every result for the requested candidate and profile.",
    "  2. Reject unavailable, failed, crashed, or timed-out attempts.",
    "  3. Require at least one output-bearing extraction.",
    "",
    "Usage:",
    "  node tools/pdf-benchmark/scripts/m5c-runtime-verdict.mjs \\",
    "    --run RUN_DIR --candidate CANDIDATE --profile PROFILE",
  ].join("\n");
}

async function verifyRuntime(input) {
  const profileRoot = resolve(input.run, input.candidate, input.profile);
  const entries = await readdir(profileRoot, { withFileTypes: true });
  const results = [];
  for (const entry of entries) {
    if (!entry.isFile() || !entry.name.endsWith(".json")) continue;
    const path = resolve(profileRoot, entry.name);
    results.push(JSON.parse(await readFile(path, "utf8")));
  }
  if (results.length === 0) {
    throw new Error(`No ${input.candidate}/${input.profile} results exist.`);
  }
  const forbidden = new Set(["unavailable", "failed", "crashed", "timeout"]);
  const failures = results.filter((result) => forbidden.has(result.outcome));
  if (failures.length > 0) {
    throw new Error(
      `${input.candidate}/${input.profile} has ${failures.length} terminal runtime failures.`,
    );
  }
  const outputBearing = new Set([
    "succeeded",
    "partial",
    "suspicious",
    "empty_success",
  ]);
  if (!results.some((result) => outputBearing.has(result.outcome))) {
    throw new Error(
      `${input.candidate}/${input.profile} produced no output-bearing result.`,
    );
  }
  process.stdout.write(
    `validated ${results.length} ${input.candidate}/${input.profile} runtime results\n`,
  );
}
