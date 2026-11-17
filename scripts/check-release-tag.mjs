import { resolve } from "node:path";
import process from "node:process";
import { fileURLToPath, URL } from "node:url";
import { setVersion, validateVersion } from "./set-version.mjs";

export function validateReleaseTag(tag, version) {
  validateVersion(version);
  if (tag !== `v${version}`) {
    throw new Error(`Release tag must be v${version}, matching package.json.`);
  }
  return tag;
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    const root = fileURLToPath(new URL("../", import.meta.url));
    const { version } = await setVersion(root, undefined, true);
    if (process.argv.length !== 3) {
      throw new Error(
        "Usage: node scripts/check-release-tag.mjs vMAJOR.MINOR.PATCH",
      );
    }
    validateReleaseTag(process.argv[2], version);
    process.stdout.write(`Release tag and manifests agree: v${version}\n`);
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
