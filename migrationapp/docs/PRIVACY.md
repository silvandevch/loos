# Privacy

LoOS Switch has **zero telemetry** as an engineering property, not a promise in
a settings screen.

- No HTTP, HTTPS, WebSocket, DNS or other network client is linked into the app.
- No analytics, crash-reporting, attribution, advertising or update SDK exists.
- The app does not create a machine ID, does not read advertising IDs, and does
  not contact any server at startup or during installation.
- Logs are held in memory. They are written to disk only when the user presses
  **Save log**. Nothing is uploaded.
- Payload verification is local: SHA-256 of the compressed and raw image plus a
  readback hash of the target.
- Migration hashes are local and are written only into the migration bundle.

To verify: run `loos-switch --self-test` with the network disconnected, watch
the process in a network monitor, or inspect the dependency tree and source
(`Cargo.toml`, `Cargo.lock`, `src/`). There is no `reqwest`, `ureq`, `hyper`,
`curl`, `tokio-net` or browser engine in the build.
