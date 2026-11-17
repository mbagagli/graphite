import { spawn } from "node:child_process";
import { dirname, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import {
  developmentPlan,
  findCudaToolkit,
  localCudaArchitectures,
} from "./native-build.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const forceCuda = args.includes("--cuda");
const plan = developmentPlan({
  root,
  platform: process.platform,
  env: process.env,
  forceCuda,
  toolkit: process.platform === "linux" ? findCudaToolkit(root) : null,
  architectures: process.platform === "linux" ? localCudaArchitectures() : null,
});
process.stdout.write(`Graphite: ${plan.message}\n`);
const child = spawn(
  process.execPath,
  [
    resolve(root, "node_modules/@tauri-apps/cli/tauri.js"),
    ...plan.args,
    ...args.filter((argument) => argument !== "--cuda"),
  ],
  {
    cwd: root,
    env: plan.env,
    stdio: "inherit",
    detached: process.platform !== "win32",
  },
);
let stopping = false;
function stop(signal = "SIGTERM") {
  if (stopping) return;
  stopping = true;
  try {
    if (process.platform === "win32") child.kill(signal);
    else process.kill(-child.pid, signal);
  } catch (error) {
    if (error.code !== "ESRCH") throw error;
  }
}
process.on("SIGINT", () => stop("SIGINT"));
process.on("SIGTERM", () => stop());
child.on("error", (error) => {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
});
child.on("exit", (code, signal) => {
  stop();
  process.exitCode =
    code ?? (signal === "SIGINT" || signal === "SIGTERM" ? 0 : 1);
});
