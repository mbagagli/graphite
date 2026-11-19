import { readFile } from "node:fs/promises";
import { URL } from "node:url";
import { describe, expect, it } from "vitest";

async function workflow(name) {
  return readFile(
    new URL(`../.github/workflows/${name}.yml`, import.meta.url),
    "utf8",
  );
}

describe("release workflow trust boundaries", () => {
  it.each(["ci", "native-compatibility", "package-smoke", "pages", "release"])(
    "pins external actions and disables stored checkout credentials in %s",
    async (name) => {
      const source = await workflow(name);
      const references = [...source.matchAll(/uses:\s*([^@\s]+)@([^\s#]+)/g)];
      expect(references.length).toBeGreaterThan(0);
      for (const [, , revision] of references) {
        expect(revision).toMatch(/^[a-f0-9]{40}$/);
      }
      expect(source).not.toContain("pull_request_target:");
      expect(source).not.toContain("secrets.");
      expect(source.match(/persist-credentials: false/g)?.length).toBe(
        references.filter(([, action]) => action === "actions/checkout").length,
      );
    },
  );

  it("keeps reusable candidate builds versioned and read-only", async () => {
    const source = await workflow("package-smoke");
    expect(source).toMatch(/^on:\n {2}workflow_dispatch:\s*$/m);
    expect(source).toContain("  workflow_call:");
    expect(source).not.toMatch(/^\s+\w[\w-]*:\s+write\s*$/m);
    expect(source).toContain("npm run package:installed-smoke");
    expect(source).toContain(
      "npm run check:publication && npm run check:privacy",
    );
  });

  it("requires validation and all native packages before creating a draft", async () => {
    const source = await workflow("release");
    const packages = await workflow("package-smoke");
    for (const platform of ["linux-x64", "macos-arm64", "windows-x64"]) {
      expect(packages).toContain(`platform: ${platform}`);
    }
    expect(packages).toContain(
      "npm run tauri -- build --ci --bundles nsis -- --locked",
    );
    expect(packages).toContain("needs: package");
    expect(source).toContain('tags:\n      - "v*"');
    expect(source).not.toContain("pull_request:");
    expect(source).toContain(
      'node scripts/check-release-tag.mjs "$RELEASE_TAG"',
    );
    expect(source).toContain('if [ "$REF_TYPE" != tag ]; then');
    expect(source).toContain("uses: ./.github/workflows/ci.yml");
    expect(source).toContain("needs: [validate, checks]");
    expect(source).toContain("uses: ./.github/workflows/package-smoke.yml");
    const draft = source.slice(source.indexOf("  draft-release:\n"));
    expect(draft).toContain("needs: [validate, packages]");
    expect(draft).toContain("if: github.ref_type == 'tag'");
    expect(draft).toContain("contents: write");
    expect(source.slice(0, source.indexOf("  draft-release:\n"))).not.toMatch(
      /^\s+\w[\w-]*:\s+write\s*$/m,
    );
    expect(draft).toContain("--verify-tag --draft");
    expect(draft).toContain("Refusing to overwrite a published release.");
    expect(draft).toContain("sha256sum --check --strict SHA256SUMS.txt");
    for (const suffix of [
      "amd64.deb",
      "amd64.AppImage",
      "aarch64.dmg",
      "x64-setup.exe",
    ]) {
      expect(draft).toContain(`Graphite_${"${VERSION}"}_${suffix}`);
    }
    expect(draft).toContain(".sourceRevision == $revision");
    expect(await workflow("ci")).toContain("  workflow_call:");
    expect(await workflow("ci")).not.toContain("npm run site:build");
    expect(await workflow("ci")).not.toContain("macos-15-intel");
    expect(await workflow("ci")).not.toContain("windows-2025");
    expect(source).not.toContain("native-compatibility.yml");
  });

  it("builds the offline archive only on explicit opt-in", async () => {
    const source = await workflow("package-smoke");
    expect(source).toContain("build_demo_archive:");
    expect(source).toContain("default: false");
    const job = source.slice(source.indexOf("  demo-archive:\n"));
    expect(job).toContain("if: inputs.build_demo_archive");
    expect(job).toContain("npm run demo:prepare -- --download");
    expect(job).toContain("npm run demo:archive");
    expect(job).not.toContain("unsigned-");
  });

  it("publishes Pages only on an explicit main-branch owner action", async () => {
    const source = await workflow("pages");
    const deploy = source.slice(source.indexOf("  deploy:\n"));
    expect(deploy).toContain(
      "if: github.event_name == 'workflow_dispatch' && inputs.publish && github.ref == 'refs/heads/main'",
    );
    expect(deploy).toContain("name: github-pages");
    expect(deploy).toContain("pages: write");
    expect(deploy).toContain("id-token: write");
    expect(source.slice(0, source.indexOf("  deploy:\n"))).not.toMatch(
      /^\s+\w[\w-]*:\s+write\s*$/m,
    );
    expect(source).toContain(
      "SITE_PUBLIC: ${{ github.event_name == 'workflow_dispatch' && inputs.publish }}",
    );
    expect(source).toContain("npm run test:site");
    expect(source).toContain(
      "npm run check:publication && npm run check:privacy",
    );
  });
});
