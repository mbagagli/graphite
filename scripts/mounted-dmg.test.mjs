import { beforeEach, expect, it, vi } from "vitest";
import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { setTimeout as sleep } from "node:timers/promises";
import { withMountedDmg } from "./mounted-dmg.mjs";

vi.mock("node:child_process", () => ({ execFileSync: vi.fn() }));
vi.mock("node:fs/promises", () => ({
  mkdir: vi.fn(),
  mkdtemp: vi.fn(),
  realpath: vi.fn(),
  rm: vi.fn(),
}));
vi.mock("node:timers/promises", () => ({ setTimeout: vi.fn() }));

const directory = join(tmpdir(), "graphite-dmg-test");
const mountPoint = join(directory, "volume");
let mounted;
let busyAttempts;
let detachCalls;
let attachError;
let inventoryError;

beforeEach(() => {
  vi.resetAllMocks();
  mounted = false;
  busyAttempts = 0;
  detachCalls = 0;
  attachError = undefined;
  inventoryError = undefined;
  mkdtemp.mockResolvedValue(directory);
  mkdir.mockResolvedValue(undefined);
  realpath.mockImplementation(async (path) => path);
  execFileSync.mockImplementation((name, args) => {
    if (name === "plutil") {
      return JSON.stringify({
        images: [
          {
            "system-entities": mounted ? [{ "mount-point": mountPoint }] : [],
          },
        ],
      });
    }
    if (args[0] === "info") {
      if (inventoryError) throw inventoryError;
      return "plist";
    }
    if (args[0] === "attach") {
      mounted = true;
      if (attachError) throw attachError;
    } else if (args[0] === "detach") {
      expect(args).toEqual(["detach", mountPoint]);
      detachCalls += 1;
      if (detachCalls <= busyAttempts) throw new Error("Resource busy");
      mounted = false;
    } else throw new Error(`Unexpected command: ${name} ${args}`);
    return "";
  });
  rm.mockImplementation(async () => {
    expect(mounted).toBe(false);
  });
});

it("inspects read-only content, detaches and then removes its directory", async () => {
  const inspect = vi.fn().mockResolvedValue("checked");
  await expect(withMountedDmg("candidate.dmg", inspect)).resolves.toBe(
    "checked",
  );
  expect(inspect).toHaveBeenCalledWith(mountPoint);
  expect(execFileSync).toHaveBeenCalledWith(
    "hdiutil",
    [
      "attach",
      "-readonly",
      "-nobrowse",
      "-mountpoint",
      mountPoint,
      "candidate.dmg",
    ],
    expect.any(Object),
  );
  expect(detachCalls).toBe(1);
  expect(rm).toHaveBeenCalledWith(directory, { recursive: true, force: true });
});

it("preserves inspection failures after successful cleanup", async () => {
  const failure = new Error("Invalid application identity");
  await expect(
    withMountedDmg("candidate.dmg", async () => {
      throw failure;
    }),
  ).rejects.toBe(failure);
  expect(rm).toHaveBeenCalledOnce();
});

it("retries a busy mount up to three times", async () => {
  busyAttempts = 2;
  await withMountedDmg("candidate.dmg", async () => {});
  expect(detachCalls).toBe(3);
  expect(sleep.mock.calls).toEqual([[1_000], [1_000]]);
  expect(rm).toHaveBeenCalledOnce();
});

it("retains a persistently busy mount and both errors", async () => {
  busyAttempts = 3;
  const failure = new Error("Inspection failed");
  const error = await withMountedDmg("candidate.dmg", async () => {
    throw failure;
  }).catch((error) => error);
  expect(error).toBeInstanceOf(AggregateError);
  expect(error.errors[0]).toBe(failure);
  expect(error.errors[1].message).toContain("retained");
  expect(error.errors[1].cause.cause.message).toBe("Resource busy");
  expect(detachCalls).toBe(3);
  expect(rm).not.toHaveBeenCalled();
});

it("fails cleanup even when inspection passed if detach stays busy", async () => {
  busyAttempts = 3;
  await expect(
    withMountedDmg("candidate.dmg", async () => {}),
  ).rejects.toBeInstanceOf(AggregateError);
  expect(rm).not.toHaveBeenCalled();
});

it("cleans up partial attachment while preserving the attach error", async () => {
  attachError = new Error("Attach failed after mounting");
  const inspect = vi.fn();
  await expect(withMountedDmg("candidate.dmg", inspect)).rejects.toBe(
    attachError,
  );
  expect(inspect).not.toHaveBeenCalled();
  expect(detachCalls).toBe(1);
  expect(rm).toHaveBeenCalledOnce();
});

it("retains the directory when mount state cannot be verified", async () => {
  inventoryError = new Error("Cannot query mounts");
  await expect(
    withMountedDmg("candidate.dmg", async () => {}),
  ).rejects.toBeInstanceOf(AggregateError);
  expect(rm).not.toHaveBeenCalled();
});

it("does not detach a volume already unmounted", async () => {
  await withMountedDmg("candidate.dmg", async () => {
    mounted = false;
  });
  expect(detachCalls).toBe(0);
  expect(rm).toHaveBeenCalledOnce();
});

it("resolves mount aliases before detaching", async () => {
  const alias = join(tmpdir(), "mount-alias");
  const originalCommand = execFileSync.getMockImplementation();
  execFileSync.mockImplementation((name, args) => {
    if (name === "plutil") {
      return JSON.stringify({
        images: [
          {
            "system-entities": mounted ? [{ "mount-point": alias }] : [],
          },
        ],
      });
    }
    return originalCommand(name, args);
  });
  realpath.mockImplementation(async (path) =>
    path === alias ? mountPoint : path,
  );
  await withMountedDmg("candidate.dmg", async () => {});
  expect(detachCalls).toBe(1);
  expect(rm).toHaveBeenCalledOnce();
});

it("preserves inspection errors if directory removal also fails", async () => {
  const inspectionError = new Error("Invalid package");
  const removalError = new Error("Cannot remove directory");
  rm.mockRejectedValue(removalError);
  const error = await withMountedDmg("candidate.dmg", async () => {
    throw inspectionError;
  }).catch((error) => error);
  expect(error.errors).toEqual([inspectionError, removalError]);
});

it("cleans an unsuccessful attach that never mounted a volume", async () => {
  attachError = new Error("Invalid image");
  const originalCommand = execFileSync.getMockImplementation();
  execFileSync.mockImplementation((name, args) => {
    if (args[0] === "attach") throw attachError;
    return originalCommand(name, args);
  });
  await expect(withMountedDmg("candidate.dmg", vi.fn())).rejects.toBe(
    attachError,
  );
  expect(detachCalls).toBe(0);
  expect(rm).toHaveBeenCalledOnce();
});

it.each(["ENOENT", "EACCES"])(
  "handles a foreign volume lookup failing with %s",
  async (code) => {
    const foreign = join(tmpdir(), "foreign-volume");
    const originalCommand = execFileSync.getMockImplementation();
    execFileSync.mockImplementation((name, args) => {
      const output = originalCommand(name, args);
      if (name !== "plutil") return output;
      const info = JSON.parse(output);
      info.images[0]["system-entities"].unshift({ "mount-point": foreign });
      return JSON.stringify(info);
    });
    realpath.mockImplementation(async (path) => {
      if (path === foreign) throw Object.assign(new Error(code), { code });
      return path;
    });
    const result = withMountedDmg("candidate.dmg", async () => {});
    if (code === "ENOENT") {
      await result;
      expect(detachCalls).toBe(1);
      expect(rm).toHaveBeenCalledOnce();
    } else {
      await expect(result).rejects.toBeInstanceOf(AggregateError);
      expect(rm).not.toHaveBeenCalled();
    }
  },
);

it("retains a missing mount reported through an alias", async () => {
  const aliasParent = join(tmpdir(), "alias-parent");
  const alias = join(aliasParent, "volume");
  const originalCommand = execFileSync.getMockImplementation();
  execFileSync.mockImplementation((name, args) => {
    const output = originalCommand(name, args);
    return name === "plutil" ? output.replaceAll(mountPoint, alias) : output;
  });
  realpath.mockImplementation(async (path) => {
    if (path === alias) {
      throw Object.assign(new Error("Mount disappeared"), { code: "ENOENT" });
    }
    return path === aliasParent ? directory : path;
  });
  await expect(
    withMountedDmg("candidate.dmg", async () => {}),
  ).rejects.toBeInstanceOf(AggregateError);
  expect(rm).not.toHaveBeenCalled();
});
