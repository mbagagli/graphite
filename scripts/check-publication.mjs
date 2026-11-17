import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath, URL } from "node:url";
import { resolve } from "node:path";
import process from "node:process";

const root = fileURLToPath(new URL("../", import.meta.url));

export function ignoredPublicationPaths(paths, directory = root) {
  if (!paths.length) return [];
  const result = spawnSync(
    "git",
    [
      "-c",
      "core.excludesFile=/dev/null",
      "check-ignore",
      "--no-index",
      "-z",
      "--stdin",
    ],
    { cwd: directory, input: paths.join("\0") + "\0", encoding: "utf8" },
  );
  if (result.error) throw result.error;
  if (![0, 1].includes(result.status))
    throw new Error("Unable to verify the public repository ignore rules.");
  return result.stdout.split("\0").filter(Boolean);
}

export function checkPublication(directory = root) {
  const paths = execFileSync("git", ["ls-files", "-z"], {
    cwd: directory,
    encoding: "utf8",
  })
    .split("\0")
    .filter(Boolean);
  if (!paths.length)
    throw new Error("No staged or tracked public source to check.");
  const ignored = ignoredPublicationPaths(paths, directory);
  if (ignored.length)
    throw new Error(
      `${ignored.length} tracked files are excluded by the public-source policy. ` +
        "Ignore rules do not remove tracked files. Review the fresh repository contents before publication.",
    );
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  checkPublication();
  process.stdout.write(
    "Public repository contains no ignored local material.\n",
  );
}
