import { chmodSync, copyFileSync, existsSync, mkdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const desktopDir = resolve(scriptDir, "..");
const repoRoot = resolve(desktopDir, "..", "..");
const targetTriple =
  process.env.TAURI_ENV_TARGET_TRIPLE ||
  process.env.CARGO_BUILD_TARGET ||
  detectHostTriple();
const binaryName = targetTriple.includes("windows") ? "agentlink.exe" : "agentlink";
const cargoTargetDir = resolveTargetDir(process.env.CARGO_TARGET_DIR);
const buildProfile = "release";
const sourceBinary = targetTriple
  ? join(cargoTargetDir, targetTriple, buildProfile, binaryName)
  : join(cargoTargetDir, buildProfile, binaryName);
const stagedBinary = join(
  desktopDir,
  "src-tauri",
  "resources-generated",
  "target",
  "release",
  binaryName
);

run(
  "cargo",
  [
    "build",
    "--manifest-path",
    join(repoRoot, "Cargo.toml"),
    "--bin",
    "agentlink",
    "--release",
    ...(targetTriple ? ["--target", targetTriple] : [])
  ],
  repoRoot
);

if (!existsSync(sourceBinary)) {
  throw new Error(`Missing built agentlink binary at ${sourceBinary}`);
}

mkdirSync(dirname(stagedBinary), { recursive: true });
copyFileSync(sourceBinary, stagedBinary);
if (extname(stagedBinary) !== ".exe") {
  chmodSync(stagedBinary, 0o755);
}

console.log(`Staged bundled agentlink: ${stagedBinary}`);

function run(command, args, cwd) {
  const result = spawnSync(command, args, {
    cwd,
    env: process.env,
    stdio: "inherit"
  });
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} exited with code ${result.status ?? "unknown"}`
    );
  }
}

function resolveTargetDir(value) {
  if (!value) {
    return join(repoRoot, "target");
  }
  return resolve(repoRoot, value);
}

function detectHostTriple() {
  const result = spawnSync("rustc", ["-vV"], {
    cwd: repoRoot,
    encoding: "utf8",
    env: process.env
  });
  if (result.status !== 0) {
    throw new Error(result.stderr || "failed to read rustc host triple");
  }
  const hostLine = result.stdout
    .split(/\r?\n/)
    .find((line) => line.startsWith("host: "));
  if (!hostLine) {
    throw new Error("rustc -vV did not report a host triple");
  }
  return hostLine.slice("host: ".length).trim();
}
