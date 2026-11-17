import { Buffer } from "node:buffer";
import { createReadStream } from "node:fs";
import { readFile, stat } from "node:fs/promises";
import { basename } from "node:path";
import { URL } from "node:url";
import { privacyFindings } from "./privacy-check.mjs";

function assertFindings(findings, packagePath) {
  if (findings.length)
    throw new Error(
      `Package privacy check failed: ${packagePath}: ${findings.join(", ")}`,
    );
}

export async function assertNoBuildIdentity(
  path,
  packagePath,
  privateBuildIdentities,
) {
  if (basename(packagePath) === "libgnutls.so.30") {
    // Hash-reviewed upstream self-tests are public fixtures, not user keys.
    // Inspect the whole bounded library so PEM blocks cannot span chunks.
    if ((await stat(path)).size > 16 * 1024 * 1024)
      throw new Error(
        "GnuTLS exceeds the reviewed inspection size; review before release.",
      );
    const review = JSON.parse(
      await readFile(
        new URL("./public-crypto-fixtures.json", import.meta.url),
        "utf8",
      ),
    );
    const digests = new Set(review.fixtures.map((fixture) => fixture.sha256));
    assertFindings(
      privacyFindings(await readFile(path), privateBuildIdentities, digests),
      packagePath,
    );
    return;
  }
  const needles = privateBuildIdentities.map((value) => Buffer.from(value));
  const overlap = Math.max(256, ...needles.map((value) => value.length)) - 1;
  let tail = Buffer.alloc(0);
  for await (const chunk of createReadStream(path)) {
    const content = Buffer.concat([tail, chunk]);
    assertFindings(
      privacyFindings(content, privateBuildIdentities),
      packagePath,
    );
    tail = content.subarray(Math.max(0, content.length - overlap));
  }
}
