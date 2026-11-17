import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { setVersion, validateVersion } from "./set-version.mjs";

const roots = [];

async function fixture() {
  const root = await mkdtemp(resolve(tmpdir(), "graphite-version-"));
  roots.push(root);
  await mkdir(resolve(root, "src-tauri"));
  const files = {
    "package.json": JSON.stringify({ version: "0.1.0" }, null, 2) + "\n",
    "package-lock.json":
      JSON.stringify(
        { version: "0.1.0", packages: { "": { version: "0.1.0" } } },
        null,
        2,
      ) + "\n",
    "src-tauri/Cargo.toml": '[package]\nname = "graphite"\nversion = "0.1.0"\n',
    "src-tauri/Cargo.lock":
      '[[package]]\nname = "graphite"\nversion = "0.1.0"\n\n[[package]]\nname = "dependency"\nversion = "0.1.0"\n',
    "src-tauri/tauri.conf.json": JSON.stringify({ version: "../package.json" }),
  };
  for (const [name, source] of Object.entries(files)) {
    await writeFile(resolve(root, name), source);
  }
  return root;
}

afterEach(async () => {
  await Promise.all(
    roots.splice(0).map((root) => rm(root, { recursive: true, force: true })),
  );
});

describe("release versions", () => {
  it.each([
    "",
    "v1.2.3",
    "01.2.3",
    "1.2",
    "1.2.3.4",
    "1.2.3-rc.1",
    "1.2.3+build",
    "1.2.3\n",
    "65536.1.0",
    "../1.2.3",
  ])("rejects invalid or unsupported version %j", (version) => {
    expect(() => validateVersion(version)).toThrow();
  });

  it("synchronizes every consumer and is idempotent", async () => {
    const root = await fixture();
    const first = await setVersion(root, "1.2.3");
    expect(first.changed).toHaveLength(4);
    expect((await setVersion(root, "1.2.3")).changed).toEqual([]);
    expect((await setVersion(root, undefined, true)).version).toBe("1.2.3");
    expect(
      await readFile(resolve(root, "src-tauri/Cargo.lock"), "utf8"),
    ).toContain('name = "dependency"\nversion = "0.1.0"');
  });

  it("rejects drift and missing inputs before writing", async () => {
    const root = await fixture();
    await writeFile(
      resolve(root, "src-tauri/Cargo.toml"),
      '[package]\nversion = "9.9.9"\n',
    );
    await expect(setVersion(root, undefined, true)).rejects.toThrow(
      "consumers disagree",
    );
    await writeFile(resolve(root, "src-tauri/Cargo.lock"), "");
    await expect(setVersion(root, "1.2.3")).rejects.toThrow("entry is missing");
    expect(
      JSON.parse(await readFile(resolve(root, "package.json"), "utf8")).version,
    ).toBe("0.1.0");
  });
});
