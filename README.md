# LemonTodo

LemonTodo is a developer-first, local-first, end-to-end encrypted Todo application.

The first product surface is a terminal TUI. A React Native mobile app is planned later, but the business logic, sync protocol, encryption envelope, and conflict model should live in a shared core.

## Product Direction

- Minimal Todo workflow for developers.
- Terminal-first interaction.
- Local-first storage and offline use.
- Multi-device sync through self-hosted or official servers.
- Server-side blind storage: task content is encrypted before upload.
- Free self-hosting.
- Official hosted service may charge for server resources.

## Documentation

- [Product MVP](docs/product-mvp.md)
- [Architecture](docs/architecture.md)
- [Licensing](docs/licensing.md)
- [Agent Guide](AGENTS.md)
- [License Notice](LICENSE.txt)
- [Trademark Guidelines](TRADEMARK_GUIDELINES.md)

## Current Development Slice

The current `dev` branch contains the local CLI/TUI foundation and the first server skeleton. The CLI/TUI binary name is `ltd`; the server binary name is `ltd-server`.

This is not the final sync product yet. It establishes the Rust workspace, core task model, SQLite persistence, JSON snapshot import/export, a local operation log, sync protocol DTOs, and basic server discovery endpoints.

## Requirements

- Rust toolchain with `cargo`
- Rust `1.95` or newer, matching the workspace `rust-version`

## Build

Build a debug binary:

```bash
cargo build -p lemontodo-tui
cargo build -p lemontodo-server
```

Build a release binary:

```bash
cargo build --release -p lemontodo-tui
cargo build --release -p lemontodo-server
```

The release binary is created at:

```bash
target/release/ltd
target/release/ltd-server
```

Run it directly:

```bash
./target/release/ltd
```

Run the server skeleton:

```bash
LEMONTODO_SERVER_HOST=0.0.0.0 LEMONTODO_SERVER_PORT=8787 cargo run -p lemontodo-server
```

Run the local end-to-end client/server test:

```bash
./scripts/e2e-local.sh
```

The script starts a temporary local server, provisions two client databases under `/tmp/lemontodo-e2e-local`, verifies first-device bootstrap, second-device connect, bidirectional sync, remote conflict inbox behavior, and session visibility, then stops the server automatically.

Run the browser registration page end-to-end test when the WASM toolchain is available:

```bash
npm install
./scripts/e2e-register-page.sh
```

This builds the registration WASM bundle, starts a temporary local server, registers through `/register` in Playwright, verifies browser login through `/`, and verifies that `ltd login` can use the created account. If `wasm-bindgen` CLI or the `wasm32-unknown-unknown` target is missing, the script exits successfully with a skip message.

Available server endpoints:

```text
GET /healthz
GET /
GET /console
GET /register
GET /register/register_wasm.js
GET /register/register_wasm_bg.wasm
GET /v1/server-info
POST /v1/account/register
POST /v1/account/login
POST /v1/account/logout
GET /v1/account/me
GET /v1/account/sessions
POST /v1/account/sessions/revoke
GET /v1/account/vault-key
PUT /v1/account/vault-key
POST /v1/sync/push
POST /v1/sync/pull
```

Server environment variables:

```text
LEMONTODO_SERVER_HOST=0.0.0.0
LEMONTODO_SERVER_PORT=8787
LEMONTODO_SERVER_DB=<platform-data-dir>/lemontodo-server/server.db
LEMONTODO_ALLOW_REGISTRATION=false
LEMONTODO_SESSION_TTL_SECS=2592000
LEMONTODO_ADMIN_EMAIL=
LEMONTODO_ADMIN_PASSWORD=
LEMONTODO_REGISTER_WASM_DIR=target/register-wasm
```

Build the browser-side account WASM bundle before using `/register` or `/`:

```bash
cargo install wasm-bindgen-cli
./scripts/build-register-wasm.sh
LEMONTODO_ALLOW_REGISTRATION=true cargo run -p lemontodo-server
```

If `rustup` is available, the script installs the `wasm32-unknown-unknown` target automatically. Otherwise install that target through your Rust toolchain manager first. The registration page derives the server auth hash and encrypted vault metadata in the browser before calling `/v1/account/register`; the login page derives the server auth hash in the browser before calling `/v1/account/login`. The master password is not posted to the server.

## Install Locally

Install `ltd` into Cargo's local binary directory:

```bash
cargo install --path crates/tui
```

After installation, make sure Cargo's bin directory is on `PATH`. Rust's official installer places Cargo tools under `~/.cargo/bin` on Linux/macOS and `%USERPROFILE%\.cargo\bin` on Windows.

Then run:

```bash
ltd
```

## Development Run

Without installing:

```bash
cargo run -p lemontodo-tui
```

Run a subcommand during development:

```bash
cargo run -p lemontodo-tui -- init
cargo run -p lemontodo-tui -- project add LemonTodo
cargo run -p lemontodo-tui -- project list
cargo run -p lemontodo-tui -- add "Ship local MVP" --project LemonTodo --tag mvp --due 2026-06-30
cargo run -p lemontodo-tui -- list --all
cargo run -p lemontodo-tui -- list --project LemonTodo --all
cargo run -p lemontodo-tui -- stats
cargo run -p lemontodo-tui -- search "mvp"
cargo run -p lemontodo-tui -- edit <task-id-prefix> "Ship edited MVP"
cargo run -p lemontodo-tui -- note <task-id-prefix> "Markdown note"
cargo run -p lemontodo-tui -- due <task-id-prefix> 2026-06-30
cargo run -p lemontodo-tui -- due <task-id-prefix>
cargo run -p lemontodo-tui -- tags <task-id-prefix> mvp terminal
cargo run -p lemontodo-tui -- export ./lemontodo.snapshot.json
cargo run -p lemontodo-tui -- import ./lemontodo.snapshot.json
cargo run -p lemontodo-tui -- ops
cargo run -p lemontodo-tui -- login --server-url http://127.0.0.1:8787 --email you@example.com
cargo run -p lemontodo-tui -- sync
cargo run -p lemontodo-tui -- sync status
cargo run -p lemontodo-tui -- logout
cargo run -p lemontodo-tui -- move <task-id-prefix> LemonTodo
cargo run -p lemontodo-tui -- done <task-id-prefix>
cargo run -p lemontodo-tui -- archive <task-id-prefix>
```

## Usage

Running `ltd` without a subcommand opens the interactive terminal UI:

```bash
ltd
```

Common CLI commands:

```bash
ltd project add LemonTodo
ltd project list
ltd add "Fix sync protocol notes" --project LemonTodo --tag sync --due 2026-06-30
ltd list --all
ltd list --project LemonTodo --all
ltd stats
ltd search sync
ltd export ./lemontodo.snapshot.json
ltd import ./lemontodo.snapshot.json
ltd ops
ltd login --server-url http://127.0.0.1:8787 --email you@example.com
ltd sync
ltd sync status
ltd sync inbox
ltd sync conflicts
ltd logout
ltd logout --all
```

Task editing commands are intended to be used inside the TUI. The old id-based root commands such as `ltd done`, `ltd edit`, `ltd note`, `ltd due`, `ltd tags`, `ltd move`, and `ltd archive` still exist for compatibility and scripts, but they are now hidden from the main CLI help.

TUI controls:

- `j` / `k` or `Up` / `Down`: select task
- `[` / `]`: switch project
- `v`: compact/detail view
- `space`: toggle done/open
- `a`: add task
- `e`: edit title
- `n`: edit note
- `d`: edit due date
- `t`: edit tags
- `m`: move to project
- `x`: archive
- `/`: search
- `c`: clear search
- `r`: refresh
- `s`: sync status
- `S`: sync now
- `?`: toggle help
- `Enter`: submit while adding
- `Esc`: close help/sync status or cancel input mode
- `q`: quit

When a sync account is configured, opening the TUI prompts once for the master password to unlock auto-sync for the current session. Press Enter at that prompt to skip auto-sync. While unlocked, the TUI syncs on startup, after local edits with a short debounce, every 60 seconds while idle, and once before quit.

## Data Location

By default, `ltd` stores local data in the platform user data directory:

```text
<data-dir>/lemontodo/lemontodo.db
```

On most Linux desktops this resolves to:

```text
~/.local/share/lemontodo/lemontodo.db
```

For account login or device connect with `--email` and no explicit `--db`, `ltd` uses an account-scoped database path:

```text
<data-dir>/lemontodo/accounts/<server-component>/<email-component>/lemontodo.db
```

Use `--db` to override the database path. This is useful for testing or keeping separate vaults:

```bash
ltd --db ./scratch.db add "Test task"
ltd --db ./scratch.db list --all
```

## Backup And Migration

Use JSON snapshots for local backup and migration:

```bash
ltd export ./lemontodo.snapshot.json
ltd import ./lemontodo.snapshot.json
```

Snapshots are plaintext local interchange files. They are not the final encrypted sync protocol.

## Local Sync Dry Run

The current `dev` branch can initialize local encrypted vault metadata, pack pending local operations into encrypted sync objects, and push those objects to a configured LemonTodo server.
Tasks and projects carry a local monotonically increasing `revision`. Each pending operation records the target object revision so the future server can store opaque encrypted objects while clients reason about ordering and conflicts.
The sync crate defines the protocol DTOs for `/v1/server-info`, `/v1/sync/push`, and `/v1/sync/pull`.

Accounts are registered on the server side. The client login flow only asks for server URL, account email, and master password. The master password is used locally to derive an auth hash for server login and to unlock the downloaded encrypted vault metadata; the master password itself is not sent as the API password field.

Initialize local vault metadata for low-level local dry-runs:

```bash
ltd vault init
```

`ltd vault` is now treated as a low-level compatibility command and hidden from the main CLI help.

Inspect local sync state:

```bash
ltd sync status
```

Log in to a server account:

```bash
ltd login --server-url http://127.0.0.1:8787 --email you@example.com
```

Run one manual sync pass for daily use:

```bash
ltd sync
```

Leave safe remote operations in the inbox instead of applying them:

```bash
ltd sync --no-apply-safe
```

Inspect and resolve pulled remote operations:

```bash
ltd sync inbox
ltd sync conflicts
ltd sync resolve <remote-operation-id-prefix> --keep-local
ltd sync resolve <remote-operation-id-prefix> --keep-remote
ltd sync apply
```

`ltd sync inbox` shows each pending remote operation with apply status, remote/local revision, local pending-op count, and skip/conflict reason when available.
`ltd sync conflicts` narrows the inbox to pending remote conflicts that need manual handling and shows the same revision context.
`ltd sync resolve ... --keep-local` marks a conflict as ignored and keeps the local state unchanged.
`ltd sync resolve ... --keep-remote` discards pending local task changes for that object and applies the remote version. This is currently limited to task conflicts.
`ltd sync` pulls and saves remote operations first, applies safe operations by default, and then pushes local pending operations. Pulling first avoids advancing the local cursor past remote changes that this device has not seen yet.
The TUI can use the same sync path automatically after a one-time per-session master password unlock. It does not store the master password or decrypted vault key after the process exits.
`ltd sync status` shows the configured server account email, whether an access token is stored locally, whether local vault metadata exists, and when possible also fetches the current remote account/session view from the server.
`ltd login` verifies the server is reachable through the login request, authenticates with an auth hash derived from the account email and master password, downloads encrypted vault metadata, and verifies that the master password can unlock it before saving local session state.
`ltd logout` clears the local token and, unless `--local-only` is used, revokes the current server session first. `ltd logout --all` revokes every active server session for the current account and then clears the local token.
When the server rejects the stored token because it is invalid or expired, sync commands now return a direct hint to run `ltd login` again.
The client sends a session device id plus a best-effort device name on login/connect. Set `LEMONTODO_DEVICE_NAME` to override the inferred hostname.
These commands prompt for the master password without echoing it to the terminal. This uses Argon2id to derive a server auth hash plus a wrapping key from the master password, decrypts the local vault key, and encrypts pending operations into sync objects.

For scripts and local development only, `--master-password` is still supported:

```bash
ltd vault init --master-password "dev-password"
ltd sync pack --master-password "dev-password" --out ./sync-pack.json
```

For low-level development, a raw vault key can still be generated and used directly:

```bash
ltd sync keygen
ltd sync pack --key <vault-key-hex> --out ./sync-pack.json
```

This is still an E2EE sync dry-run. It can log in with a derived auth hash to obtain a local access token, inspect the current authenticated account/session, list and revoke active sessions/devices, connect a new device by verifying downloaded vault metadata with the master password, revoke the current session, enforce server-configured session TTL expiry, upload and download encrypted vault metadata, bootstrap an admin from server environment variables, upload encrypted objects with authenticated user isolation, decrypt pulled objects, store them in a local pending-apply inbox, and apply safe remote creates plus safe task updates/archives. It does not implement full conflict resolution yet.
Use `ltd ops` to inspect pending local operations and their object revisions.
After a successful local or scripted upload simulation, mark uploaded operations as synced:

```bash
ltd sync ack --all-pending --cursor <server-cursor>
ltd sync ack <operation-id-prefix> --cursor <server-cursor>
```

## Current Limitations

- Remote push exists; remote pull stores pending remote operations, and apply handles safe remote creates plus safe task updates/archives.
- Server has `/healthz`, `/v1/server-info`, `/v1/account/register`, `/v1/account/login`, `/v1/account/logout`, `/v1/account/me`, `/v1/account/sessions`, `/v1/account/sessions/revoke`, `/v1/account/vault-key`, and token-authenticated `/v1/sync/push` and `/v1/sync/pull`; session TTL expiry is configurable, but richer device/session management is not implemented yet.
- OS keyring support is not implemented yet.
- `ltd ops` only inspects the local pending operation log; `ltd sync ack` remains useful for manual dry-runs, while `ltd sync push` acknowledges accepted server uploads automatically.
