import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { InferenceControls } from "./InferenceControls";
import { contextLevels, contextTokens } from "./inferenceSettings";
import { browserAgentEnvironment } from "../../adapters/demo/browserDiagnostics";

describe("inference controls", () => {
  it("keeps four distinct bounded context sizes for both profiles", () => {
    expect(
      contextLevels.map(({ value }) => contextTokens(value, 8192)),
    ).toEqual([2048, 4096, 6144, 8192]);
    expect(
      contextLevels.map(({ value }) => contextTokens(value, 16384)),
    ).toEqual([4096, 8192, 12288, 16384]);
  });
  it("defaults to Medium and disables GPU without a usable device", () => {
    const environment = browserAgentEnvironment;
    const markup = renderToStaticMarkup(
      <InferenceControls
        settings={environment.settings}
        hardware={{ ...environment.hardware, compiledBackend: "CPU" }}
        capacity={8192}
        onSettings={() => {}}
      />,
    );
    expect(markup).toContain('aria-valuetext="Medium, 4,096 tokens"');
    expect(markup).toMatch(/disabled=""[^>]*value="gpu"/);
    expect(markup).toContain("CPU-only build");
  });
});
