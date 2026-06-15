# LemonTodo Architecture

## High-Level Architecture

```text
TUI client
React Native client later
  |
  v
Shared core
  - model
  - local storage
  - encryption envelope
  - sync protocol
  - merge/conflict handling
  |
  v
Self-hosted or official sync server
  - auth
  - encrypted object storage
  - change log
  - device metadata
```

## First Implementation Bias

Prefer Rust for the first implementation:

- TUI quality and distribution are strong.
- A single binary is good for developer tools.
- Crypto, SQLite, sync, and file/system integration are mature.
- The same core can later be exposed to React Native through an FFI boundary.

The repository should evolve toward:

```text
crates/core      model, sync state, merge logic
crates/crypto    E2EE envelope and key handling
crates/sync      protocol client and server-neutral sync logic
crates/tui       terminal UI
crates/server    self-hosted sync server
apps/mobile      future React Native shell
docs/            product, architecture, protocol, licensing
```

## Repository Strategy

Start with a monorepo.

Do not split server, TUI, mobile, protocol, and shared core into independent GitHub repositories during MVP development. The sync protocol, encryption envelope, server behavior, and clients will change together early on, so a monorepo keeps compatibility work explicit and testable.

The long-term asset is the protocol, not the number of repositories.

Required boundaries inside the monorepo:

- protocol documentation
- shared model and sync types
- shared crypto envelope
- server implementation
- client implementations
- end-to-end compatibility tests

Future repository splits may be considered only after:

- the sync protocol reaches a stable v1
- client and server release schedules are meaningfully independent
- maintainership differs by component
- CI, permissions, or commercial module access require separation
- compatibility tests can still run across released artifacts

Potential future split:

```text
lemontodo-protocol
lemontodo-core
lemontodo-server
lemontodo-tui
lemontodo-mobile
lemontodo-commercial
```

Unofficial forks may modify any component according to the applicable licenses, but official clients should only guarantee compatibility with servers implementing the documented LemonTodo Sync Protocol.

Servers should expose a capability endpoint, for example:

```http
GET /v1/server-info
```

Example response:

```json
{
  "protocol_version": "0.1",
  "server": "lemontodo-server",
  "features": ["sync.v1", "invite.v1"],
  "limits": {
    "max_object_size": 262144,
    "max_batch_size": 100
  }
}
```

## Local Storage

Use SQLite locally.

Suggested local tables:

- tasks
- lists
- tags or task_tags
- operations
- sync_state
- conflicts

The current local implementation includes `tasks`, `lists`, `operations`, and `sync_state`. `operations` and `sync_state` are schema placeholders for the upcoming encrypted sync layer and are not yet used for remote synchronization.

Local backup and migration should use the versioned JSON snapshot format exposed by `ltd export` and `ltd import`. This is a developer-facing interchange format, not the final encrypted sync protocol.

Do not use Markdown files as the only source of truth. Markdown should be a note field and an import/export format.

## Task Model

Baseline task shape:

```text
Task {
  id: uuid
  list_id: uuid
  title: string
  note_markdown: string
  status: open | done | archived
  tags: string[]
  due_date?: date
  sort_key: string
  created_at: timestamp
  updated_at: timestamp
  deleted_at?: timestamp
  field_versions: map
}
```

## Sync Model

Use encrypted object sync with a server-side change log.

Object types:

- vault
- list
- task
- tombstone
- device

The server must not understand Todo content. It stores encrypted objects and enough metadata to support sync.

Server-visible metadata may include:

- user id
- vault id
- object id
- object version
- object size
- updated timestamp
- device id

Server-hidden content must include:

- task title
- task note
- list name
- tags
- due date
- status

## Conflict Strategy

MVP should optimize for not losing data.

Use field-aware merge where practical:

- Different task changed on different devices: merge.
- Different fields on the same task changed: merge.
- Same field changed concurrently: conflict.
- Delete versus edit: preserve data and create conflict.

Do not use last-write-wins for destructive conflicts.

Do not implement CRDT for MVP.

## Encryption Model

The server is blind storage. Encryption and decryption happen on the client.

Recommended key hierarchy:

```text
master password
  -> Argon2id
  -> key-encryption key

random vault key
  -> encrypts task/list/device objects
```

Object encryption should use an AEAD scheme such as XChaCha20-Poly1305 or AES-256-GCM.

Each encrypted object needs:

- object id
- vault id
- version
- cipher suite identifier
- nonce
- ciphertext
- authenticated associated data as needed

The server must never receive the master password or plaintext vault key.

## Account And Master Password

Follow the Bitwarden-style separation:

- Account password authenticates to the server.
- Master password unlocks the encrypted vault key.

Registration flow:

1. User creates account.
2. Client generates random vault key.
3. Client derives key-encryption key from master password.
4. Client encrypts vault key.
5. Server stores encrypted vault key and sync metadata.

New device flow:

1. User logs into server account.
2. Client downloads encrypted vault metadata.
3. User enters master password.
4. Client decrypts vault key locally.
5. Client syncs encrypted objects and decrypts locally.

Forgotten master password cannot be recovered by the server.

## Self-Hosted Server

First version should be a conventional self-hosted service, not a Cloudflare-specific implementation.

Recommended stack:

- Rust HTTP server, or another single-binary backend if later chosen.
- SQLite for small self-hosted instances.
- Postgres for production deployments.
- Local filesystem blob storage for MVP.
- S3-compatible object storage as an optional backend later.
- Docker Compose as the primary deployment path.

Minimum server responsibilities:

- user auth
- session/token management
- admin bootstrap
- registration/invite policy
- encrypted vault metadata
- encrypted object storage
- change log and cursors
- device metadata

The server must not:

- decrypt user data
- search user tasks
- merge plaintext task fields
- allow admins to read user content
- reset a user's master password

## Server Configuration

Self-hosted instances must be configurable through environment variables.

Baseline environment variables:

```env
LEMONTODO_HTTP_ADDR=0.0.0.0:8080
LEMONTODO_BASE_URL=https://todo.example.com
LEMONTODO_DATABASE_URL=sqlite:///data/lemontodo.db
LEMONTODO_STORAGE_DRIVER=local
LEMONTODO_STORAGE_PATH=/data/blobs

LEMONTODO_SIGNUPS_ALLOWED=false
LEMONTODO_INVITES_ALLOWED=true

LEMONTODO_ADMIN_EMAIL=admin@example.com
LEMONTODO_ADMIN_PASSWORD_FILE=/run/secrets/admin_password
LEMONTODO_JWT_SECRET_FILE=/run/secrets/jwt_secret
```

Plain password environment variables may be supported for local development, but file-based secrets should be documented for production.

Admin bootstrap behavior:

- If no admin exists, create the admin from environment/config.
- If an admin already exists, do not overwrite credentials on restart.
- Admins can manage users and invites, but cannot inspect encrypted user data.

## Registration Policy

Support these modes:

- Public registration enabled.
- Public registration disabled.
- Invite-only registration.

Default self-hosted behavior should be conservative:

```env
LEMONTODO_SIGNUPS_ALLOWED=false
LEMONTODO_INVITES_ALLOWED=true
```

## Cloudflare Decision

Cloudflare can support an official hosted implementation later with Workers, D1, R2, Durable Objects, Queues, and Turnstile.

However, first version should not depend on Cloudflare because the product is self-host-first. The sync protocol must remain platform-neutral.

Cloudflare should be treated as one possible official hosting implementation, not the product architecture.
