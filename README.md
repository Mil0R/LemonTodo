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

The current `dev` branch contains the first local-only CLI/TUI foundation. The installed binary name is `ltd`.

```bash
cargo run -p lemontodo-tui -- init
cargo run -p lemontodo-tui -- add "Ship local MVP" --tag mvp --due 2026-06-30
cargo run -p lemontodo-tui -- list --all
cargo run -p lemontodo-tui -- search "mvp"
cargo run -p lemontodo-tui -- edit <task-id-prefix> "Ship edited MVP"
cargo run -p lemontodo-tui -- note <task-id-prefix> "Markdown note"
cargo run -p lemontodo-tui -- due <task-id-prefix> 2026-06-30
cargo run -p lemontodo-tui -- due <task-id-prefix>
cargo run -p lemontodo-tui -- tags <task-id-prefix> mvp terminal
cargo run -p lemontodo-tui -- export ./lemontodo.snapshot.json
cargo run -p lemontodo-tui -- import ./lemontodo.snapshot.json
cargo run -p lemontodo-tui -- ops
cargo run -p lemontodo-tui -- done <task-id-prefix>
cargo run -p lemontodo-tui -- archive <task-id-prefix>
```

This is not the final interactive TUI yet. It establishes the Rust workspace, core task model, SQLite persistence, and command surface that the TUI will build on.

Running `ltd` without a subcommand opens the interactive terminal UI:

```bash
cargo run -p lemontodo-tui
```

Initial TUI controls:

- `j` / `Down`: move down
- `k` / `Up`: move up
- `space`: toggle selected task done/open
- `a`: add a task
- `e`: edit selected task title
- `n`: edit selected task note
- `d`: edit selected task due date
- `t`: edit selected task tags
- `/`: search tasks
- `c`: clear search
- `x`: archive selected task
- `Enter`: submit task while adding
- `Esc`: cancel input mode or quit from browse mode
- `r`: refresh
- `q`: quit
