import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { lstat, readFile, readdir } from "node:fs/promises";
import { homedir, hostname } from "node:os";
import { dirname, resolve, relative } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const identities = [root, homedir(), hostname()].filter(
  (value) => value.length >= 4,
);

export function containsVersionControlBackup(names) {
  return names.some((name) =>
    /(?:^|\/)objects\/(?:[a-f0-9]{2}\/[a-f0-9]{38}|pack\/pack-[a-f0-9]+\.(?:pack|idx))$/.test(
      name,
    ),
  );
}

// Return categories only: a failed check must not print the sensitive value.
export function privacyFindings(
  content,
  privateIdentities = identities,
  publicPemDigests = new Set(),
) {
  const text = content.toString();
  const findings = [];
  if (
    privateIdentities.some(
      (value) =>
        text.includes(value) || text.includes(value.replaceAll("\\", "/")),
    )
  )
    findings.push("local build identity");
  if (
    /(?<![A-Za-z0-9])(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{35,}|sk-(?:proj-)?[A-Za-z0-9_-]{25,}|AKIA[0-9A-Z]{16})/.test(
      text,
    )
  )
    findings.push("credential-shaped token");
  // Crypto libraries contain PEM labels; only flag labels with key material.
  const pemText = text
    .replaceAll("\\r\\n", "\n")
    .replaceAll("\\n", "\n")
    .replace(
      /-----BEGIN ((?:[A-Z]+ )?PRIVATE KEY)-----[A-Za-z0-9+/= \t\r\n]+-----END \1-----/g,
      (pem) =>
        publicPemDigests.has(createHash("sha256").update(pem).digest("hex"))
          ? ""
          : pem,
    );
  if (
    /-----BEGIN (?:[A-Z]+ )?PRIVATE KEY-----[ \t\r\n]*(?:(?:Proc-Type|DEK-Info):[^\r\n]*\r?\n)*[ \t\r\n]*[A-Za-z0-9+/]{16}/.test(
      pemText,
    )
  )
    findings.push("private key");
  return findings;
}

async function checkFile(path) {
  const findings = privacyFindings(await readFile(path));
  if (findings.length)
    throw new Error(`${relative(root, path)}: ${findings.join(", ")}`);
}

export async function checkPublicDirectory(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name);
    if (entry.isSymbolicLink())
      throw new Error("Public output must not contain symbolic links.");
    if (entry.isDirectory()) await checkPublicDirectory(path);
    else if (entry.isFile()) await checkFile(path);
  }
}

async function main() {
  const directories = process.argv.slice(2);
  if (directories.length) {
    for (const directory of directories)
      await checkPublicDirectory(resolve(root, directory));
  } else {
    const names = execFileSync("git", ["ls-files", "-z"], { cwd: root })
      .toString()
      .split("\0")
      .filter(Boolean);
    if (containsVersionControlBackup(names))
      throw new Error(
        "Tracked version-control backup detected. Exclude archived Git metadata from the public source, including a fresh repository.",
      );
    for (const name of names) {
      // Local-only material is outside the inspection boundary.
      if (
        name
          .split("/")
          .some((part) =>
            /^(?:DNS|DoNotStage|__original__$|__legacy)/.test(part),
          )
      )
        continue;
      const path = resolve(root, name);
      let info;
      try {
        info = await lstat(path);
      } catch (error) {
        if (error.code === "ENOENT") continue;
        throw error;
      }
      if (info.isSymbolicLink())
        throw new Error("Tracked symbolic links require release review.");
      if (info.isFile()) await checkFile(path);
    }
  }
  process.stdout.write(
    "Privacy check passed: no local build identities or credential signatures detected.\n",
  );
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    await main();
  } catch (error) {
    let message = error.message;
    for (const identity of identities)
      message = message.replaceAll(identity, "[private location]");
    process.stderr.write(`Privacy check failed: ${message}\n`);
    process.exitCode = 1;
  }
}
