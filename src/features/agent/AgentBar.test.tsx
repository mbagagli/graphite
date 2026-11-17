import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import {
  browserAgentEnvironment,
  browserWorkspace,
} from "../../adapters/demo/browserDiagnostics";
import { assistantDisplayName } from "../../shared/productIdentity";
import { AgentBar, SetupPanel } from "./AgentBar";

it("starts with an accessible launcher instead of an intrusive composer", () => {
  const html = renderToStaticMarkup(
    <AgentBar
      semantic={browserWorkspace.semantic}
      availableTextDocuments={0}
      sourceMode="demo"
      onLibrarySetup={() => {}}
      selection={{ paperIds: [], source: "graph" }}
      activePaper={null}
      activeLens={null}
      activeCollectionId={null}
      view="graph"
      readerContext={null}
      prompt={null}
      onPromptConsumed={() => {}}
      onRunComplete={() => {}}
      onOpenSource={() => {}}
      onCreateTemporaryLens={() => {}}
    />,
  );
  expect(html).toContain(`aria-label="Open ${assistantDisplayName}"`);
  expect(html).toContain('aria-expanded="false"');
  expect(html).toContain(assistantDisplayName.toUpperCase());
  expect(html).not.toContain('name="research-agent-query"');
  expect(html).not.toContain('class="agent-workspace"');
  expect(html).not.toContain('class="agent-preparation"');
});

it.each(["graphite-managed", "openai-compatible"] as const)(
  "explains the %s choice and keeps credentials specific to the local server",
  (providerKind) => {
    const html = renderToStaticMarkup(
      <SetupPanel
        environment={browserAgentEnvironment}
        settings={{ ...browserAgentEnvironment.settings, providerKind }}
        apiKey=""
        installingProfile={null}
        installMessage={null}
        providerCheck={null}
        error={null}
        settingsStatus={null}
        settingsError={null}
        onSettings={() => {}}
        onDraftSettings={() => {}}
        onApiKey={() => {}}
        onInstall={() => {}}
        onSave={() => {}}
        onCheck={() => {}}
        onUnload={() => {}}
      />,
    );
    expect(html).toContain("Advanced: answer model");
    expect(html).toContain("Graphite finds relevant passages");
    expect(html).toContain("Built-in model (recommended)");
    expect(html).toContain("Another app on this computer");
    expect(html).toContain("GPT and Claude cloud APIs are not supported");
    if (providerKind === "openai-compatible") {
      expect(html).toContain('name="loopback-endpoint"');
      const credentialInput = html.match(
        /<input\b[^>]*\bname="session-credential"[^>]*>/,
      )?.[0];
      expect(credentialInput).toContain('type="password"');
      expect(html).toContain('aria-describedby="agent-local-server-key-help"');
      expect(html).toContain("does not enable cloud access here");
      expect(html).toContain("it does not connect Graphite to OpenAI");
    } else {
      expect(html).not.toContain('name="session-credential"');
    }
  },
);
