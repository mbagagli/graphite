import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import process from "node:process";
import { spawn } from "node:child_process";
import { access } from "node:fs/promises";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { launchUntilReady } from "./installed-package-smoke.mjs";

vi.mock("node:child_process", async (original) => ({
  ...(await original()),
  spawn: vi.fn(),
}));
vi.mock("node:fs/promises", async (original) => ({
  ...(await original()),
  access: vi.fn(),
}));
vi.mock("node:timers/promises", () => ({
  setTimeout: (delay) =>
    new Promise((resolve) => globalThis.setTimeout(resolve, delay)),
}));

let child;
let ready;
let kill;
const layout = { databasePath: "test.sqlite3", childEnvironment: {} };

beforeEach(() => {
  vi.useFakeTimers();
  ready = false;
  child = Object.assign(new EventEmitter(), {
    pid: 12345,
    exitCode: null,
    signalCode: null,
    stdout: new PassThrough(),
    stderr: new PassThrough(),
  });
  spawn.mockReturnValue(child);
  access.mockImplementation(async (path) => {
    if (path === layout.databasePath && !ready) {
      throw Object.assign(new Error("Not created"), { code: "ENOENT" });
    }
  });
  kill = vi.spyOn(process, "kill").mockImplementation(() => {
    child.signalCode = "SIGTERM";
    return true;
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

async function start() {
  const result = launchUntilReady("graphite", layout).catch((error) => error);
  await vi.advanceTimersByTimeAsync(0);
  child.stdout.write("startup output");
  child.stderr.write("startup diagnostics");
  return { result };
}

it("reports spawn errors without killing a nonexistent process", async () => {
  const { result } = await start();
  child.pid = undefined;
  const cause = new Error("EACCES");
  child.emit("error", cause);
  await vi.advanceTimersByTimeAsync(250);
  const error = await result;
  expect(error.cause).toBe(cause);
  expect(error.message).toContain("startup diagnostics");
  expect(kill).not.toHaveBeenCalled();
});

it("reports early exit with captured output", async () => {
  const { result } = await start();
  child.exitCode = 1;
  await vi.advanceTimersByTimeAsync(250);
  expect((await result).message).toContain("startup output");
  expect(kill).not.toHaveBeenCalled();
});

it("includes both output streams on timeout and stops the process", async () => {
  const { result } = await start();
  await vi.advanceTimersByTimeAsync(45_000);
  const error = await result;
  expect(error.message).toContain("45000 ms");
  expect(error.message).toContain("startup output");
  expect(error.message).toContain("startup diagnostics");
  expect(kill).toHaveBeenCalledOnce();
});

it("accepts a stable startup then terminates its process", async () => {
  ready = true;
  const { result } = await start();
  await vi.advanceTimersByTimeAsync(3_000);
  expect(await result).toBeUndefined();
  expect(kill).toHaveBeenCalledOnce();
});

it("preserves startup and termination failures together", async () => {
  const { result } = await start();
  const cleanupError = new Error("Cannot terminate");
  kill.mockImplementation(() => {
    throw cleanupError;
  });
  child.emit("error", new Error("Process failure"));
  await vi.advanceTimersByTimeAsync(250);
  const error = await result;
  expect(error).toBeInstanceOf(AggregateError);
  expect(error.errors[0].message).toContain("Process failure");
  expect(error.errors[1]).toBe(cleanupError);
});
