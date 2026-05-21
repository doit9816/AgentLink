## 1. Updater feed implementation

- [x] 1.1 Switch the desktop updater endpoint to the public static feed URL.
- [x] 1.2 Add a script that stages a public updater feed from signed bundle output and rewrites metadata URLs.
- [x] 1.3 Update the release workflow to publish the staged updater feed for tagged Windows releases.

## 2. Documentation and validation

- [x] 2.1 Update release/update documentation to describe the public updater feed and deployment expectation.
- [x] 2.2 Run focused validation for the changed workflow surface (`cargo check` as needed, `npm run build:ui`, and script sanity checks without packaging).
