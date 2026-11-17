import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import process from "node:process";
import { setTimeout as delay } from "node:timers/promises";
import { URL } from "node:url";

import { artifactDirectory, startWebDriver } from "./qa-runtime.mjs";

const url = new URL(
  process.env.GRAPHITE_PAGES_PREVIEW || "http://127.0.0.1:4173/graphite/",
);
assert.equal(url.hostname, "127.0.0.1", "Pages QA must remain loopback-only.");
const output = artifactDirectory("pages");
await mkdir(output, { recursive: true });
const manifest = JSON.parse(
  await readFile(new URL("../demo/manifest.json", import.meta.url), "utf8"),
);
const driver = await startWebDriver();
let sessionId;
const verification = { base: url.pathname, viewports: [] };

async function evaluate(script) {
  return driver.request("POST", `/session/${sessionId}/execute/sync`, {
    script,
    args: [],
  });
}

try {
  const session = await driver.createFirefoxSession();
  sessionId = session.sessionId;
  for (const page of [
    { path: "", name: "landing", title: "Graphite - Discover Knowledge" },
    { path: "manual.html", name: "manual", title: "User manual - Graphite" },
    {
      path: "tutorial.html",
      name: "tutorial",
      title: "Your first investigation - Graphite",
    },
  ]) {
    const pageUrl = new URL(page.path, url);
    for (const viewport of [
      { label: "desktop", width: 1440, height: 900 },
      { label: "mobile", width: 390, height: 844 },
    ]) {
      await driver.request(
        "POST",
        `/session/${sessionId}/window/rect`,
        viewport,
      );
      process.stdout.write(`Checking ${page.name} ${viewport.label}\n`);
      let frame;
      if (viewport.label === "mobile") {
        // A frame avoids the desktop browser's minimum window width.
        await driver.request("POST", `/session/${sessionId}/url`, {
          url: "about:blank",
        });
        frame = await evaluate(`
        document.body.style.margin = '0';
        const frame = document.createElement('iframe');
        frame.style.cssText = 'display:block;border:0;width:${viewport.width}px;height:${viewport.height}px';
        frame.src = ${JSON.stringify(pageUrl.href)};
        document.body.appendChild(frame);
        return frame;
      `);
        await driver.request("POST", `/session/${sessionId}/frame`, {
          id: frame,
        });
      } else {
        await driver.request("POST", `/session/${sessionId}/url`, {
          url: pageUrl.href,
        });
      }
      await evaluate(
        "for (const image of document.images) image.loading = 'eager';",
      );
      let loaded = false;
      for (let attempt = 0; attempt < 50; attempt += 1) {
        loaded =
          await evaluate(`return document.title === ${JSON.stringify(page.title)} &&
        document.fonts.status === 'loaded' &&
        [...document.images].every(image => image.complete && image.naturalWidth > 0);`);
        if (loaded) break;
        await delay(100);
      }
      assert.ok(loaded, "All local page assets must finish loading.");
      const state = await evaluate(`
      const links = [...document.querySelectorAll('a')];
      const missingAnchors = links.filter(link => link.getAttribute('href').startsWith('#'))
        .filter(link => !document.getElementById(link.hash.slice(1))).map(link => link.hash);
      const localUrls = [...document.querySelectorAll('a[href],link[href],img[src]')]
        .map(node => node.href || node.src)
        .filter(value => new URL(value).origin === location.origin && !new URL(value).hash);
      const externalResources = performance.getEntriesByType('resource')
        .map(resource => resource.name).filter(value => new URL(value).origin !== location.origin);
      return { missingAnchors, localUrls, externalResources,
        horizontalOverflow: document.documentElement.scrollWidth > innerWidth,
        attributionCount: document.querySelectorAll('.attribution li').length,
        title: document.title, height: document.documentElement.scrollHeight,
        innerWidth, foreground: getComputedStyle(document.body).color,
        background: getComputedStyle(document.documentElement).backgroundColor };
    `);
      assert.deepEqual(state.missingAnchors, []);
      assert.deepEqual(state.externalResources, []);
      assert.equal(state.horizontalOverflow, false);
      assert.equal(state.innerWidth, viewport.width);
      assert.equal(
        state.attributionCount,
        page.name === "landing" ? manifest.papers.length : 0,
      );
      assert.equal(state.title, page.title);
      for (const href of new Set(state.localUrls)) {
        assert.equal(
          (await globalThis.fetch(href)).status,
          200,
          "A local Pages link is broken.",
        );
      }
      const luminance = (color) => {
        const channels = color
          .match(/\d+/g)
          .slice(0, 3)
          .map(Number)
          .map((value) => {
            const channel = value / 255;
            return channel <= 0.04045
              ? channel / 12.92
              : ((channel + 0.055) / 1.055) ** 2.4;
          });
        return (
          channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722
        );
      };
      const contrast =
        (luminance(state.foreground) + 0.05) /
        (luminance(state.background) + 0.05);
      assert.ok(contrast >= 4.5, "Body text contrast must meet WCAG AA.");
      const captures = [];
      if (frame) {
        const maximumScroll = Math.max(0, state.height - viewport.height);
        for (let offset = 0; ; offset += viewport.height) {
          const scrollY = Math.min(offset, maximumScroll);
          const actualScroll = await evaluate(`
          document.documentElement.style.scrollBehavior = 'auto';
          window.scrollTo({ top: ${scrollY}, behavior: 'instant' });
          return window.scrollY;
        `);
          assert.ok(Math.abs(actualScroll - scrollY) < 1);
          await delay(100);
          await driver.request("POST", `/session/${sessionId}/frame`, {
            id: null,
          });
          await driver.request("POST", `/session/${sessionId}/window/rect`, {
            width: 800,
            height: viewport.height + 200,
          });
          await evaluate("window.scrollTo(0, 0);");
          const frameId = frame["element-6066-11e4-a52e-4f735466cecf"];
          const png = await driver.request(
            "GET",
            `/session/${sessionId}/element/${frameId}/screenshot`,
          );
          const screenshot = Buffer.from(png, "base64");
          assert.equal(screenshot.readUInt32BE(16), viewport.width);
          assert.equal(screenshot.readUInt32BE(20), viewport.height);
          const filename =
            captures.length === 0
              ? `${page.name}-mobile.png`
              : `${page.name}-mobile-${captures.length + 1}.png`;
          await writeFile(resolve(output, filename), screenshot);
          captures.push({ filename, scrollY: actualScroll });
          if (scrollY === maximumScroll) break;
          await driver.request("POST", `/session/${sessionId}/frame`, {
            id: frame,
          });
        }
      } else {
        const png = await driver.request(
          "GET",
          `/session/${sessionId}/moz/screenshot/full`,
        );
        await writeFile(
          resolve(output, `${page.name}-desktop.png`),
          Buffer.from(png, "base64"),
        );
        captures.push({ filename: `${page.name}-desktop.png`, scrollY: 0 });
      }
      verification.viewports.push({
        page: page.name,
        ...viewport,
        measuredWidth: state.innerWidth,
        documentHeight: state.height,
        captures,
        bodyContrast: contrast,
        localLinks: new Set(state.localUrls).size,
      });
    }
  }
  await writeFile(
    resolve(output, "verification.json"),
    `${JSON.stringify(verification, null, 2)}\n`,
  );
  process.stdout.write(
    "Pages smoke passed: desktop/mobile, navigation, local assets, attribution, and body contrast.\n",
  );
} finally {
  if (sessionId)
    await driver.request("DELETE", `/session/${sessionId}`).catch(() => {});
  await driver.stop();
}
