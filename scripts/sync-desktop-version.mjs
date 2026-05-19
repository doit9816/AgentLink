import fs from "node:fs";
import path from "node:path";

const version = process.argv[2];

if (!version) {
  console.error("Usage: node scripts/sync-desktop-version.mjs <version>");
  process.exit(1);
}

if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) {
  console.error(`Invalid version: ${version}`);
  process.exit(1);
}

const root = process.cwd();

function updateJsonFile(relativePath, mutate) {
  const filePath = path.join(root, relativePath);
  const data = JSON.parse(fs.readFileSync(filePath, "utf8"));
  mutate(data);
  fs.writeFileSync(filePath, `${JSON.stringify(data, null, 2)}\n`);
}

function updatePackageVersion(relativePath) {
  const filePath = path.join(root, relativePath);
  const source = fs.readFileSync(filePath, "utf8");
  const updated = source.replace(
    /^(\[package\]\s*(?:\r?\n(?:(?!^\[).*))?\r?\nversion = ")([^"]+)(")/m,
    `$1${version}$3`
  );

  if (updated === source) {
    throw new Error(`Could not update package version in ${relativePath}`);
  }

  fs.writeFileSync(filePath, updated);
}

updateJsonFile("client/agentlink-desktop/package.json", (data) => {
  data.version = version;
});

updateJsonFile("client/agentlink-desktop/package-lock.json", (data) => {
  data.version = version;
  if (data.packages?.[""]) {
    data.packages[""].version = version;
  }
});

updateJsonFile("client/agentlink-desktop/src-tauri/tauri.conf.json", (data) => {
  data.version = version;
});

updatePackageVersion("client/agentlink-desktop/src-tauri/Cargo.toml");

console.log(`Synchronized desktop version to ${version}`);
