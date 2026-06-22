# AGENTS.md

This file guides coding agents working on LemonTodo.

## Product Constraints

LemonTodo is a developer-first, local-first, end-to-end encrypted Todo app.

The first user interface is a terminal TUI. A Web client is a supported second surface for project/task workflows. A React Native app may be added later, but business logic must remain in a shared core.

Do not expand the MVP into a general notes app, project-management suite, Notion clone, or Joplin clone.

## Current Decisions

- Self-hosted server first.
- Cloudflare is not part of the first implementation.
- Official hosted service may be added later and may charge for server resources.
- Self-hosting must remain free.
- Optional billing is disabled by default. When disabled, all accounts are Premium. When enabled, new non-admin accounts default to Free and only Premium accounts can create projects beyond Inbox.
- Server-side storage is blind: clients encrypt before upload.
- Server administrators must not be able to read user Todo content.
- Account password is for server authentication.
- Master password is for decrypting the vault key.
- Forgotten master password is not recoverable by the server.
- Use a Bitwarden-style licensing model: GPLv3 clients, AGPLv3 server, source-available commercial modules only if needed later.

## Technical Direction

Prefer Rust for the initial implementation unless a later project decision changes this.

Use a monorepo during MVP development. Do not split server, TUI, mobile, protocol, and shared core into separate GitHub repositories until the sync protocol is stable and release/maintainer boundaries justify it.

Expected long-term layout:

```text
crates/core
crates/crypto
crates/sync
crates/tui
apps/web
crates/server
apps/mobile
docs
```

Use SQLite locally. Use SQLite or Postgres on the self-hosted server. Use local filesystem blob storage first, with S3-compatible storage as a later option.

## MVP Scope

Implement first:

- Inbox.
- Lists/projects.
- Task title.
- Task status: open, done, archived.
- Markdown note field.
- Tags.
- Optional due date.
- Local search.
- Manual sync.
- Conflict view.
- Admin bootstrap for self-hosted server.
- Registration policy controlled by environment/config.
- Web client for project/task workflows with session-scoped auto sync.

Do not implement in MVP:

- rich text
- attachments
- calendar UI
- complex recurring tasks
- team collaboration
- server-side search
- plugin system
- CRDT collaboration

## Sync Rules

Use encrypted object sync with a server-side change log.

The documented LemonTodo Sync Protocol is the compatibility boundary. Official clients should only guarantee compatibility with servers that implement the same protocol version and advertised capabilities.

Servers should eventually expose `/v1/server-info` with protocol version, feature flags, and limits.

The server can store object ids, versions, sizes, timestamps, device ids, and encrypted blobs. It must not need plaintext task fields.

Protocol MVP endpoints:

- `GET /v1/server-info` returns protocol version, server capabilities, and batch/object limits.
- `POST /v1/sync/push` accepts encrypted sync objects, the client device id, and the client's last known server cursor.
- `POST /v1/sync/pull` accepts the client device id, last known server cursor, and object limit; it returns newer encrypted objects plus the next cursor.

The server cursor is opaque to clients. Clients store it as `sync.last_cursor` and must not parse it.

The server may reject individual pushed objects without rejecting the whole batch. Rejections must be explicit and machine-readable.

Conflict handling must prefer data preservation:

- merge independent object changes
- merge independent field changes where practical
- create conflicts for concurrent same-field edits
- preserve delete-versus-edit conflicts
- avoid destructive last-write-wins

## Encryption Rules

Encryption and decryption happen on the client.

Recommended design:

```text
master password
  -> Argon2id
  -> key-encryption key

random vault key
  -> encrypts Todo objects
```

Use an AEAD cipher such as XChaCha20-Poly1305 or AES-256-GCM.

The server must never receive the master password or plaintext vault key.

## Self-Hosted Server Rules

The server must support:

- public registration enabled
- public registration disabled
- invite-only registration
- admin bootstrap through environment/config

Admin bootstrap must only create the first admin when no admin exists. Restarting the server must not overwrite an existing admin password.

Prefer file-based secrets for production:

```env
LEMONTODO_ADMIN_PASSWORD_FILE=/run/secrets/admin_password
LEMONTODO_JWT_SECRET_FILE=/run/secrets/jwt_secret
```

Plain password environment variables may exist only for local development convenience.

## Licensing Rules

Preserve the Bitwarden-style model:

- client code: GPLv3
- server code: AGPLv3
- selected future commercial modules: source-available LemonTodo License
- no trademark grant from code licenses

Do not describe a non-commercial source-available license as open source.

GPLv3 and AGPLv3 allow commercial use when their obligations are followed.

## Documentation Rules

Update docs when changing product or architecture decisions:

- `docs/product-mvp.md`
- `docs/architecture.md`
- `docs/licensing.md`
- `AGENTS.md`

Keep protocol-level decisions explicit. The sync protocol is a long-term product asset and should not be tied to a single hosting provider.
