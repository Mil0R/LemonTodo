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
LEMONTODO_SERVER_HOST=127.0.0.1 LEMONTODO_SERVER_PORT=8787 cargo run -p lemontodo-server
```

Available server endpoints:

```text
GET /healthz
GET /v1/server-info
POST /v1/account/register
POST /v1/account/login
POST /v1/account/logout
GET /v1/account/me
GET /v1/account/vault-key
PUT /v1/account/vault-key
POST /v1/sync/push
POST /v1/sync/pull
```

Server environment variables:

```text
LEMONTODO_SERVER_HOST=127.0.0.1
LEMONTODO_SERVER_PORT=8787
LEMONTODO_SERVER_DB=<platform-data-dir>/lemontodo-server/server.db
LEMONTODO_ALLOW_REGISTRATION=false
LEMONTODO_SESSION_TTL_SECS=2592000
LEMONTODO_ADMIN_EMAIL=
LEMONTODO_ADMIN_PASSWORD=
```

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
cargo run -p lemontodo-tui -- sync configure --server-url http://127.0.0.1:8787 --email you@example.com
cargo run -p lemontodo-tui -- sync register --email you@example.com
cargo run -p lemontodo-tui -- sync login --email you@example.com
cargo run -p lemontodo-tui -- sync connect --email you@example.com
cargo run -p lemontodo-tui -- sync connect --email you@example.com --pull
cargo run -p lemontodo-tui -- sync connect --email you@example.com --pull --apply-safe
cargo run -p lemontodo-tui -- sync whoami
cargo run -p lemontodo-tui -- sync logout
cargo run -p lemontodo-tui -- sync vault push
cargo run -p lemontodo-tui -- sync vault pull
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
ltd edit <task-id-prefix> "Fix encrypted sync protocol notes"
ltd note <task-id-prefix> "Markdown note"
ltd due <task-id-prefix> 2026-06-30
ltd due <task-id-prefix>
ltd tags <task-id-prefix> sync mvp
ltd done <task-id-prefix>
ltd archive <task-id-prefix>
ltd export ./lemontodo.snapshot.json
ltd import ./lemontodo.snapshot.json
ltd ops
ltd vault init
ltd vault status
ltd sync pack --out ./sync-pack.json
ltd sync configure --server-url http://127.0.0.1:8787 --email you@example.com
ltd sync register --email you@example.com
ltd sync login --email you@example.com
ltd sync connect --email you@example.com
ltd sync connect --email you@example.com --pull
ltd sync connect --email you@example.com --pull --apply-safe
ltd sync whoami
ltd sync status
ltd sync logout
ltd sync vault push
ltd sync vault pull
ltd move <task-id-prefix> LemonTodo
```

TUI controls:

- `j` / `Down`: select next task
- `k` / `Up`: select previous task
- `[` / `]`: switch project filter
- `v`: switch compact/detail view
- `space`: toggle selected task done/open
- `a`: add a task
- `e`: edit selected task title
- `n`: edit selected task note
- `d`: edit selected task due date
- `t`: edit selected task tags
- `m`: move selected task to a project
- `/`: search tasks
- `c`: clear search
- `x`: archive selected task
- `?`: show or hide full help
- `Enter`: submit task while adding
- `Esc`: close help or cancel input mode
- `r`: refresh
- `q`: quit

## Data Location

By default, `ltd` stores local data in the platform user data directory:

```text
<data-dir>/lemontodo/lemontodo.db
```

On most Linux desktops this resolves to:

```text
~/.local/share/lemontodo/lemontodo.db
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

Initialize local vault metadata:

```bash
ltd vault init
```

Create an encrypted sync pack:

```bash
ltd sync pack --out ./sync-pack.json
```

Inspect local sync state:

```bash
ltd sync status
```

Configure a sync server and push pending encrypted operations:

```bash
ltd sync configure --server-url http://127.0.0.1:8787 --email you@example.com
ltd sync register --email you@example.com
ltd sync login --email you@example.com
ltd sync vault push
ltd sync push
```

Connect a new device to an existing account:

```bash
ltd sync configure --server-url http://127.0.0.1:8787 --email you@example.com
ltd sync connect --email you@example.com
```

Connect a new device and pull the first batch of remote operations into the local inbox:

```bash
ltd sync configure --server-url http://127.0.0.1:8787 --email you@example.com
ltd sync connect --email you@example.com --pull
```

Connect a new device, pull the first batch, and immediately apply only safe remote operations:

```bash
ltd sync configure --server-url http://127.0.0.1:8787 --email you@example.com
ltd sync connect --email you@example.com --pull --apply-safe
```

Pull and decrypt remote operations without applying them locally:

```bash
ltd sync pull
ltd sync inbox
ltd sync conflicts
ltd sync resolve <remote-operation-id-prefix> --keep-local
ltd sync resolve <remote-operation-id-prefix> --keep-remote
ltd sync apply
ltd sync pull --json
```

`ltd sync inbox` shows each pending remote operation with apply status, remote/local revision, local pending-op count, and skip/conflict reason when available.
`ltd sync conflicts` narrows the inbox to pending remote conflicts that need manual handling and shows the same revision context.
`ltd sync resolve ... --keep-local` marks a conflict as ignored and keeps the local state unchanged.
`ltd sync resolve ... --keep-remote` discards pending local task changes for that object and applies the remote version. This is currently limited to task conflicts.
`ltd sync status` shows the configured server account email, whether an access token is stored locally, whether local vault metadata exists, and when possible also fetches the current remote account/session view from the server.
`ltd sync whoami` calls the server with the stored access token and shows which account and session the server currently sees, including whether encrypted vault metadata exists remotely.
`ltd sync logout` clears the local token and, unless `--local-only` is used, revokes the current server session first.
`ltd sync vault push` uploads the local `EncryptedVaultKey` to the current server account. `ltd sync vault pull` downloads it for a new device and refuses to overwrite local metadata unless `--force` is passed.
`ltd sync connect` is the recommended new-device onboarding command: it logs into the server account, downloads encrypted vault metadata, verifies that the provided master password can unlock it locally, and only then saves the local token and metadata. With `--pull`, it immediately downloads the first batch of encrypted remote operations into the local inbox. With `--pull --apply-safe`, it also runs the same safe automatic apply path as `ltd sync apply`, leaving skipped and conflicting items in the inbox.
When the server rejects the stored token because it is invalid or expired, sync commands now return a direct hint to run `ltd sync login` again.
These commands prompt for the master password without echoing it to the terminal. Registration and login also prompt for the account password without echoing it. This uses Argon2id to derive a wrapping key from the master password, decrypts the local vault key, and encrypts pending operations into sync objects.

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

This is still an E2EE sync dry-run. It can register password-based server accounts, log in to obtain a local access token, inspect the current authenticated account/session, connect a new device by verifying downloaded vault metadata with the master password, revoke the current session, enforce server-configured session TTL expiry, upload and download encrypted vault metadata, bootstrap an admin from server environment variables, upload encrypted objects with authenticated user isolation, decrypt pulled objects, store them in a local pending-apply inbox, and apply safe remote creates plus safe task updates/archives. It does not implement full conflict resolution yet.
Use `ltd ops` to inspect pending local operations and their object revisions.
After a successful local or scripted upload simulation, mark uploaded operations as synced:

```bash
ltd sync ack --all-pending --cursor <server-cursor>
ltd sync ack <operation-id-prefix> --cursor <server-cursor>
```

## Current Limitations

- Remote push exists; remote pull stores pending remote operations, and apply handles safe remote creates plus safe task updates/archives.
- Server has `/healthz`, `/v1/server-info`, `/v1/account/register`, `/v1/account/login`, `/v1/account/logout`, `/v1/account/me`, `/v1/account/vault-key`, and token-authenticated `/v1/sync/push` and `/v1/sync/pull`; session TTL expiry is configurable, but richer device/session management is not implemented yet.
- OS keyring support is not implemented yet.
- `ltd ops` only inspects the local pending operation log; `ltd sync ack` remains useful for manual dry-runs, while `ltd sync push` acknowledges accepted server uploads automatically.
