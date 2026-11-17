import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const detachAttempts = 3;
const detachDelayMs = 1_000;

function command(name, args, input) {
  return execFileSync(name, args, {
    encoding: "utf8",
    timeout: 30_000,
    stdio: [input === undefined ? "ignore" : "pipe", "pipe", "pipe"],
    input,
  });
}

async function isMounted(mountPoint) {
  const plist = command("hdiutil", ["info", "-plist"]);
  const info = JSON.parse(
    command("plutil", ["-convert", "json", "-o", "-", "-"], plist),
  );
  if (!Array.isArray(info.images)) {
    throw new Error("Cannot verify DMG mounts: invalid hdiutil inventory.");
  }
  for (const image of info.images) {
    for (const entity of image["system-entities"] ?? []) {
      const path = entity["mount-point"];
      // Resolve macOS /var aliases before comparing mount identities.
      if (!path) continue;
      if (path === mountPoint) return true;
      try {
        if ((await realpath(path)) === mountPoint) return true;
      } catch (error) {
        // A foreign volume may disappear while reading the inventory.
        // Our mount's parent can still resolve an alias of a missing path.
        if (error.code !== "ENOENT") throw error;
        if ((await realpath(mountPoint)) !== mountPoint) throw error;
        const parent = await realpath(dirname(path)).catch((parentError) => {
          if (parentError.code !== "ENOENT") throw parentError;
          return null;
        });
        if (parent && join(parent, basename(path)) === mountPoint) throw error;
      }
    }
  }
  return false;
}

async function detach(mountPoint) {
  let lastError;
  for (let attempt = 0; attempt < detachAttempts; attempt += 1) {
    if (!(await isMounted(mountPoint))) return;
    try {
      command("hdiutil", ["detach", mountPoint]);
    } catch (error) {
      lastError = error;
    }
    if (!(await isMounted(mountPoint))) return;
    if (attempt + 1 < detachAttempts) await sleep(detachDelayMs);
  }
  throw new Error("DMG remains mounted after three detach attempts.", {
    cause: lastError,
  });
}

/** Inspect a read-only DMG and remove its mount directory only once detached. */
export async function withMountedDmg(dmg, inspect) {
  const directory = await realpath(
    await mkdtemp(join(tmpdir(), "graphite-dmg-")),
  );
  const mountPoint = join(directory, "volume");
  const errors = [];
  let result;
  try {
    await mkdir(mountPoint);
    command("hdiutil", [
      "attach",
      "-readonly",
      "-nobrowse",
      "-mountpoint",
      mountPoint,
      dmg,
    ]);
    result = await inspect(mountPoint);
  } catch (error) {
    errors.push(error);
  }
  try {
    // Also handles attach failing after the volume was mounted.
    await detach(mountPoint);
  } catch (error) {
    errors.push(
      new Error(
        `DMG cleanup could not verify unmounting; retained ${directory}.`,
        { cause: error },
      ),
    );
    throw new AggregateError(errors, "DMG inspection or cleanup failed.", {
      cause: error,
    });
  }
  try {
    await rm(directory, { recursive: true, force: true });
  } catch (error) {
    errors.push(error);
  }
  if (errors.length === 1) throw errors[0];
  if (errors.length > 1) {
    throw new AggregateError(errors, "DMG inspection and cleanup failed.");
  }
  return result;
}
