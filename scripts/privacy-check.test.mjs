import { Buffer } from "node:buffer";
import { createHash } from "node:crypto";
import { expect, it } from "vitest";
import {
  containsVersionControlBackup,
  privacyFindings,
} from "./privacy-check.mjs";

it("detects identities and credentials without returning their values", () => {
  const identity = "/example/private-checkout";
  expect(privacyFindings(identity, [identity])).toEqual([
    "local build identity",
  ]);
  const token = "gh" + "p_" + "x".repeat(36);
  expect(privacyFindings(token, [])).toEqual(["credential-shaped token"]);
  expect(
    privacyFindings("https://doi.org/10.5281/zenodo.24282715", []),
  ).toEqual([]);
  expect(privacyFindings("http://127.0.0.1:23119", [])).toEqual([]);
});

it.each(["", "RSA ", "DSA ", "EC ", "OPENSSH ", "ENCRYPTED "])(
  "detects %sprivate key material, including escaped and truncated PEM",
  (type) => {
    const header = "-----BEGIN " + type + "PRIVATE KEY-----";
    const body = "A".repeat(64);
    for (const newline of ["", "\n", "\r\n", "\\n", "\\r\\n"]) {
      expect(privacyFindings(header + newline + body, [])).toEqual([
        "private key",
      ]);
    }
    expect(privacyFindings(header + "\0", [])).toEqual([]);
    expect(privacyFindings(header + "\n%s\n", [])).toEqual([]);
    expect(
      privacyFindings(
        header +
          "\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-256-CBC,00\n\n" +
          body,
        [],
      ),
    ).toEqual(["private key"]);
  },
);

it("allows only exact reviewed PEM fixtures without hiding other findings", () => {
  const pem =
    "-----BEGIN " +
    "PRIVATE KEY-----\n" +
    "A".repeat(64) +
    "\n-----END " +
    "PRIVATE KEY-----";
  const digests = new Set([createHash("sha256").update(pem).digest("hex")]);
  expect(privacyFindings(pem, [])).toEqual(["private key"]);
  expect(privacyFindings(pem, [], digests)).toEqual([]);
  expect(privacyFindings(pem.replace("AAAA", "BBBB"), [], digests)).toEqual([
    "private key",
  ]);
  expect(privacyFindings(pem + "\n" + pem.slice(0, 80), [], digests)).toEqual([
    "private key",
  ]);
  expect(
    privacyFindings(pem + "\nprivate-identity", ["private-identity"], digests),
  ).toEqual(["local build identity"]);
  expect(privacyFindings(pem + "\nsk-" + "X".repeat(32), [], digests)).toEqual([
    "credential-shaped token",
  ]);
});

it("does not mistake a GTK namespaced symbol for a credential", () => {
  expect(
    privacyFindings(Buffer.from("\0gsk-serialization-error-quark\0"), []),
  ).toEqual([]);
});

it.each(["sk-", "sk-proj-", "ghp_", "github_pat_", "AKIA"])(
  "still detects %s tokens at common value boundaries",
  (prefix) => {
    const token = prefix + "A".repeat(prefix === "AKIA" ? 16 : 40);
    for (const wrapper of [
      token,
      `key=${token}`,
      `"${token}"`,
      `Bearer ${token}`,
      `\0${token}\0`,
    ]) {
      expect(privacyFindings(Buffer.from(wrapper), [])).toEqual([
        "credential-shaped token",
      ]);
    }
  },
);

it("blocks archived Git objects from public source without reading them", () => {
  expect(
    containsVersionControlBackup(["backup/objects/ab/" + "c".repeat(38)]),
  ).toBe(true);
  expect(containsVersionControlBackup(["src/objects/reader.ts"])).toBe(false);
});
