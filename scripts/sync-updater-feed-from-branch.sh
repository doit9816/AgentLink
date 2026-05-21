#!/usr/bin/env bash
# One-off helper: copy the legacy updater-feed branch into main/updater-feed/ and rewrite URLs.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="${GITHUB_REPOSITORY:-doit9816/AgentLink}"
OLD="https://raw.githubusercontent.com/${REPO}/updater-feed"
NEW="https://raw.githubusercontent.com/${REPO}/main/updater-feed"

platforms=(
  "darwin/aarch64"
  "darwin/x86_64"
  "windows/x86_64"
  "linux/x86_64"
)

for plat in "${platforms[@]}"; do
  dest="${ROOT}/updater-feed/${plat}"
  mkdir -p "${dest}"

  curl -fsSL "${OLD}/${plat}/latest.json" -o "${dest}/latest.json"

  artifact="$(
    python3 - <<PY
import json
with open("${dest}/latest.json", encoding="utf-8") as f:
    data = json.load(f)
print(next(iter(data["platforms"].values()))["url"].rsplit("/", 1)[-1])
PY
  )"

  curl -fsSL "${OLD}/${plat}/${artifact}" -o "${dest}/${artifact}"
  curl -fsSL "${OLD}/${plat}/${artifact}.sig" -o "${dest}/${artifact}.sig"

  python3 - <<PY
import json
path = "${dest}/latest.json"
with open(path, encoding="utf-8") as f:
    data = json.load(f)
for entry in data.get("platforms", {}).values():
    name = entry["url"].rsplit("/", 1)[-1]
    entry["url"] = "${NEW}/${plat}/" + name
with open(path, "w", encoding="utf-8") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
PY

  echo "synced ${plat} (${artifact})"
done
