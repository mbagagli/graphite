import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir } from "node:fs/promises";
import { createServer } from "node:net";
import { dirname, resolve } from "node:path";
import process from "node:process";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const SCRIPT_DIRECTORY = dirname(fileURLToPath(import.meta.url));
const REPOSITORY_ROOT = resolve(SCRIPT_DIRECTORY, "..");
const LOOPBACK_HOST = "127.0.0.1";
const QA_APP_URL = `http://${LOOPBACK_HOST}:1420`;
const PROBE_TIMEOUT_MS = 1_000;
const WEBDRIVER_COMMAND_TIMEOUT_MS = 30_000;

export function artifactDirectory(suiteName, environment = process.env) {
  const configuredRoot = environment.GRAPHITE_QA_ARTIFACTS_DIR?.trim();
  const artifactRoot = configuredRoot
    ? resolve(REPOSITORY_ROOT, configuredRoot)
    : resolve(REPOSITORY_ROOT, "artifacts", "qa");
  return resolve(artifactRoot, suiteName);
}

export function firefoxOptions(environment = process.env) {
  const binary = environment.GRAPHITE_QA_FIREFOX_BINARY?.trim();
  return {
    args: ["-headless"],
    ...(binary ? { binary } : {}),
  };
}

export function parsePerformanceArguments(args) {
  const parsed = parseArgs({
    args,
    allowPositionals: true,
    options: {
      large: { type: "boolean", default: false },
    },
    strict: true,
  });
  if (parsed.positionals.length > 1) {
    throw new Error("Performance QA accepts at most one profile label.");
  }
  const profileLabel = parsed.positionals[0] ?? "current";
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(profileLabel)) {
    throw new Error(
      "The performance profile label must be a safe 1-64 character " +
        "filename using letters, numbers, dots, underscores, or hyphens.",
    );
  }
  return {
    largeGraphMode: parsed.values.large,
    profileLabel,
  };
}

export function formatDriverStartupError(executable, error, stderr = "") {
  const diagnostic = stderr.trim();
  const detail = diagnostic ? ` Driver output: ${diagnostic}` : "";
  return new Error(
    `Unable to start Geckodriver '${executable}': ${error.message}. ` +
      "Install geckodriver on PATH or set GRAPHITE_QA_GECKODRIVER to its " +
      `executable path.${detail}`,
  );
}

async function availableLoopbackPort() {
  const server = createServer();
  server.unref();
  await new Promise((accept, reject) => {
    server.once("error", reject);
    server.listen({ host: LOOPBACK_HOST, port: 0, exclusive: true }, accept);
  });
  const address = server.address();
  const port = typeof address === "object" && address ? address.port : null;
  await new Promise((accept, reject) =>
    server.close((error) => {
      if (error) reject(error);
      else accept();
    }),
  );
  if (port === null) {
    throw new Error("Unable to allocate a loopback WebDriver port.");
  }
  return port;
}

function configuredDriverPort(environment) {
  const value = environment.GRAPHITE_QA_DRIVER_PORT?.trim();
  if (!value) return null;
  const port = Number(value);
  if (!Number.isInteger(port) || port < 1 || port > 65_535) {
    throw new Error(
      "GRAPHITE_QA_DRIVER_PORT must be an integer between 1 and 65535.",
    );
  }
  return port;
}

function requestClient(driverUrl) {
  return async (method, path, body) => {
    const timeout =
      path === "/status" ? PROBE_TIMEOUT_MS : WEBDRIVER_COMMAND_TIMEOUT_MS;
    const response = await globalThis.fetch(`${driverUrl}${path}`, {
      method,
      headers: { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: globalThis.AbortSignal.timeout(timeout),
    });
    const responseText = await response.text();
    let payload;
    try {
      payload = JSON.parse(responseText);
    } catch {
      throw new Error(
        `WebDriver ${method} ${path} returned a non-JSON response.`,
      );
    }
    if (!response.ok || payload.value?.error) {
      throw new Error(
        `WebDriver ${method} ${path} failed: ` + JSON.stringify(payload.value),
      );
    }
    return payload.value;
  };
}

async function waitForApplication() {
  for (let attempt = 0; attempt < 30; attempt += 1) {
    try {
      const response = await globalThis.fetch(QA_APP_URL, {
        signal: globalThis.AbortSignal.timeout(PROBE_TIMEOUT_MS),
      });
      if (response.ok) return;
    } catch {
      // The loop below provides a bounded retry for local server startup.
    }
    await delay(250);
  }
  throw new Error(
    `Graphite QA could not reach ${QA_APP_URL}. Start 'npm run dev' in ` +
      "another terminal before running browser QA.",
  );
}

async function waitForDriver(request) {
  for (let attempt = 0; attempt < 40; attempt += 1) {
    try {
      await request("GET", "/status");
      return;
    } catch {
      // The driver process race below reports early process failures.
    }
    await delay(250);
  }
  throw new Error("Firefox WebDriver did not become ready within 10 seconds.");
}

async function stopChildProcess(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGTERM");
  const exited = await Promise.race([
    once(child, "exit").then(() => true),
    delay(3_000).then(() => false),
  ]);
  if (!exited && child.exitCode === null) {
    child.kill("SIGKILL");
    await once(child, "exit").catch(() => {});
  }
}

export async function startWebDriver(environment = process.env) {
  const executable =
    environment.GRAPHITE_QA_GECKODRIVER?.trim() || "geckodriver";
  const port =
    configuredDriverPort(environment) ?? (await availableLoopbackPort());
  const driverUrl = `http://${LOOPBACK_HOST}:${port}`;
  const request = requestClient(driverUrl);
  const child = spawn(
    executable,
    ["--host", LOOPBACK_HOST, "--port", String(port)],
    {
      shell: false,
      stdio: ["ignore", "ignore", "pipe"],
      windowsHide: true,
    },
  );
  let ready = false;
  let stderr = "";
  child.stderr?.on("data", (chunk) => {
    stderr = `${stderr}${chunk.toString()}`.slice(-4_096);
  });
  const processFailure = new Promise((_, reject) => {
    child.once("error", (error) => {
      reject(formatDriverStartupError(executable, error, stderr));
    });
    child.once("exit", (code, signal) => {
      if (!ready) {
        const reason = new Error(
          `process exited early with code ${String(code)} and signal ` +
            `${String(signal)}`,
        );
        reject(formatDriverStartupError(executable, reason, stderr));
      }
    });
  });

  try {
    await Promise.race([waitForDriver(request), processFailure]);
    ready = true;
  } catch (error) {
    await stopChildProcess(child);
    throw error;
  }

  return {
    driverUrl,
    request,
    async createFirefoxSession() {
      try {
        return await request("POST", "/session", {
          capabilities: {
            alwaysMatch: {
              browserName: "firefox",
              "moz:firefoxOptions": firefoxOptions(environment),
            },
          },
        });
      } catch (error) {
        throw new Error(
          `${error.message} Install Firefox or set ` +
            "GRAPHITE_QA_FIREFOX_BINARY to its executable path.",
          { cause: error },
        );
      }
    },
    stop: () => stopChildProcess(child),
  };
}

export async function prepareQaRuntime(suiteName, environment = process.env) {
  const outputDirectory = artifactDirectory(suiteName, environment);
  await mkdir(outputDirectory, { recursive: true });
  await waitForApplication();
  const driver = await startWebDriver(environment);
  return {
    ...driver,
    appUrl: QA_APP_URL,
    outputDirectory,
  };
}

export async function assertBrowserQaIsolation(request, sessionId) {
  const state = await request("POST", `/session/${sessionId}/execute/sync`, {
    script: `
      const externalResources = performance.getEntriesByType("resource")
        .map((entry) => entry.name)
        .filter((resource) => {
          try {
            return new URL(resource).origin !== window.location.origin;
          } catch {
            return false;
          }
        });
      return {
        externalResources,
        hasTauriRuntime: "__TAURI_INTERNALS__" in window,
        hasDemoSource: document.body.textContent.includes("Demo source"),
        hostname: window.location.hostname,
      };
    `,
    args: [],
  });
  if (
    state.hostname !== LOOPBACK_HOST ||
    state.hasTauriRuntime ||
    !state.hasDemoSource ||
    state.externalResources.length > 0
  ) {
    throw new Error(`Browser QA isolation failed: ${JSON.stringify(state)}.`);
  }
  return state;
}
