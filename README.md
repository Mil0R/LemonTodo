# LemonTodo

LemonTodo is a local-first, end-to-end encrypted Todo app with a terminal client, Web client, and self-hosted sync server.

## Quick Start

Compiled `ltd` release binaries can be downloaded directly from GitHub Releases:

`https://github.com/Mil0R/LemonTodo/releases`

After downloading, place `ltd` on your `PATH` and run:

```bash
ltd
```

## Docker Compose Deployment

Use [compose.yaml](/home/lin/Workspace/LemonTodo/compose.yaml) to self-host the server and bundled Web client.

1. Prepare the environment file:

```bash
cp .env.example .env
```

2. Set the basic values in `.env`:

```env
LEMONTODO_IMAGE=ghcr.io/mil0r/lemontodo:latest
LEMONTODO_PORT=8787
LEMONTODO_ALLOW_REGISTRATION=true
LEMONTODO_ADMIN_EMAIL=
```

3. Start the service:

```bash
docker compose up -d
docker compose logs -f
```

4. Open:

- Web client: `http://<host>:<port>/`
- Registration page: `http://<host>:<port>/register`
- Health check: `http://<host>:<port>/healthz`

Data is stored at `/data/lemontodo.db` inside the container and persisted in the Docker volume `lemontodo-data`.

Upgrade the deployment:

```bash
docker compose pull
docker compose up -d
```

Stop the deployment without deleting data:

```bash
docker compose down
```

## Administrator Initialization

`LEMONTODO_ADMIN_EMAIL` only promotes an existing registered account.

Initialize the administrator in this order:

1. Set `LEMONTODO_ALLOW_REGISTRATION=true` and leave `LEMONTODO_ADMIN_EMAIL` empty.
2. Run `docker compose up -d`.
3. Register the future administrator account through Web or `ltd register`.
4. Set `LEMONTODO_ALLOW_REGISTRATION=false`.
5. Set `LEMONTODO_ADMIN_EMAIL` to that registered email.
6. Run:

```bash
docker compose up -d --force-recreate
```

7. Confirm the logs contain `Promoted registered account ... to administrator`.

## Web Client

- Open `http://<host>:<port>/`
- Log in with your account email and master password
- If registration is enabled, create the first account at `http://<host>:<port>/register`

## `ltd` Quick Start

```bash
ltd
```

```bash
ltd register --server-url http://127.0.0.1:8787 --email you@example.com
ltd login --server-url http://127.0.0.1:8787 --email you@example.com
ltd sync
ltd sync status
ltd logout
```

Use a custom local database path for testing or multiple vaults:

```bash
ltd --db ./scratch.db
```

## Common CLI Commands

```bash
ltd project list
ltd project add LemonTodo
ltd add "Fix sync issue" --project LemonTodo --tag sync --due 2026-07-31
ltd list --all
ltd list --project LemonTodo --all
ltd search sync
ltd stats
ltd export ./lemontodo.snapshot.json
ltd import ./lemontodo.snapshot.json
ltd ops
ltd sync
ltd sync status
ltd sync inbox
ltd sync conflicts
ltd logout
ltd logout --all
```

## TUI Keys

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
- `Esc`: cancel input mode or close overlays
- `q`: quit

## Local Data

Default local database:

```text
<data-dir>/lemontodo/lemontodo.db
```

On most Linux desktops:

```text
~/.local/share/lemontodo/lemontodo.db
```

Account-scoped login without `--db` uses:

```text
<data-dir>/lemontodo/accounts/<server-component>/<email-component>/lemontodo.db
```

## Backup

```bash
ltd export ./lemontodo.snapshot.json
ltd import ./lemontodo.snapshot.json
```
