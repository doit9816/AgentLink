# Desktop updater feed

Public static files for the Tauri desktop auto-updater.

- **URL pattern:** `https://raw.githubusercontent.com/doit9816/AgentLink/main/updater-feed/{{target}}/{{arch}}/latest.json`
- **Layout:** `<target>/<arch>/latest.json`, installer/archive, and `.sig` per platform
- **Published by:** `.github/workflows/release.yml` on tagged releases (`v*`)

Do not edit `latest.json` or binaries by hand during release; CI regenerates this tree from signed bundle output.
