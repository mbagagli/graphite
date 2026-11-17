import { beforeEach, expect, it, vi } from "vitest";
import { createTauriServices } from "./TauriServices";

const invoke = vi.hoisted(() => vi.fn(async () => ({})));
vi.mock("@tauri-apps/api/core", () => ({
  invoke,
  Channel: class {},
  convertFileSrc: vi.fn((id: string) => `graphite-document://localhost/${id}`),
}));
beforeEach(() => invoke.mockClear());

it("binds PDF reads to the revision returned by native resolution", async () => {
  invoke.mockResolvedValueOnce({
    status: "ready",
    revisionKey: "verified-hash",
  });
  const document = await createTauriServices("demo").document.resolve("paper");
  expect(document.url).toBe(
    "graphite-document://localhost/paper?revision=verified-hash",
  );
  expect(invoke).toHaveBeenCalledWith("resolve_document", {
    documentId: "paper",
  });
});

it("activates the demo partition without touching Zotero synchronization", async () => {
  const service = createTauriServices("demo").workspace;
  await service.load();
  await service.load();
  expect(invoke.mock.calls).toEqual([["activate_demo_library"]]);
  await service.sync();
  await service.rebuild();
  expect(invoke.mock.calls).toEqual([
    ["activate_demo_library"],
    ["sync_document_corpus"],
    ["rebuild_document_index"],
  ]);
});

it("restores the Zotero partition before the usual local synchronization", async () => {
  const service = createTauriServices("zotero").workspace;
  await service.load();
  await service.sync();
  expect(invoke.mock.calls).toEqual([
    ["activate_zotero_library"],
    ["sync_zotero"],
    ["sync_document_corpus"],
  ]);
});

it("persists lenses through native commands and invalidates cached workspaces", async () => {
  const services = createTauriServices("demo");
  await services.workspace.load();
  const request = {
    libraryId: "demo",
    name: "Lens",
    description: "Question",
    paperIds: ["one"],
  };
  await services.lenses.save(request);
  await services.workspace.load();
  await services.lenses.list("demo");
  await services.lenses.delete("demo", "lens-one", 2);
  await services.workspace.load();
  expect(invoke.mock.calls).toEqual([
    ["activate_demo_library"],
    ["save_lens", { request }],
    ["activate_demo_library"],
    ["list_lenses", { libraryId: "demo" }],
    ["delete_lens", { libraryId: "demo", id: "lens-one", revision: 2 }],
    ["activate_demo_library"],
  ]);
});
