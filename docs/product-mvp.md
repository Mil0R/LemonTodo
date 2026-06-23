# LemonTodo Product MVP

## Positioning

LemonTodo is not a general notes app, Notion replacement, Joplin clone, or team project-management system.

The MVP is:

- A developer-first Todo List.
- TUI-first.
- Web second.
- Mobile planned with React Native and Expo.
- Local-first.
- End-to-end encrypted when synced.
- Self-host friendly.
- Compatible with an official hosted service later.

## Core MVP Features

- Inbox as the default task list.
- Lists or projects for simple grouping.
- Task title.
- Task status: open, done, archived.
- Markdown note field per task.
- Tags using simple `#tag` semantics.
- Optional due date using date-only values.
- Local search.
- Manual sync first, automatic sync later.
- Automatic sync should exist in both TUI and Web as a session-scoped feature, not a background daemon.
- Clear sync status: pending, synced, offline, conflict.
- Conflict view with explicit user resolution.

## Explicit Non-Goals For MVP

- Rich text editing.
- Attachments.
- Calendar UI.
- Complex recurring tasks.
- Team collaboration.
- Server-side search.
- Plugin system.
- CRDT-based real-time collaboration.
- Markdown vault as the only source of truth.

## Web Client MVP Scope

The Web client is a focused second surface, not a full product rewrite.

It should cover:

- Project list and project switching.
- Task creation.
- Task completion toggle.
- Task title editing.
- Task note editing.
- Task due date editing.
- Task tag editing.
- Task move between projects.
- Task archive.
- Search and filter.
- Session-scoped auto sync.

It should not start with:

- Internal sync debugging views.
- Key management screens.
- Raw protocol inspection.
- Backend admin tools.

## Mobile Client MVP Scope

The mobile client should be a focused Android and iOS surface for the same project/task workflows as the TUI and Web client.

It should cover:

- Server URL setup.
- Account login and registration when allowed by the server.
- Registration collects email, master password, and master password confirmation after server URL validation.
- Master password unlock.
- Project list as the main screen.
- Task list per project.
- Fast task creation with title only.
- Task detail editing for note, due date, and tags.
- Task completion, archive, and delete.
- Encrypted sync using the same protocol as TUI and Web.
- Free and Premium account behavior from `/v1/account/me`.

It should not start with:

- Mobile-only sync semantics.
- Background daemon sync.
- Internal sync debugging screens.
- Hosted-service-only flows.
- Complex project-management features.

Free mobile users can use Inbox but cannot create, rename, archive, or delete projects. Premium mobile users can manage projects. Task creation remains available inside allowed projects.

## TUI Interaction Baseline

The TUI should optimize for keyboard-driven developer workflows.

```text
j/k       move selection
a         add task
e         edit task
space     toggle done
p         move project/list
/         search
s         sync now
c         open conflict view
:         command mode
```

These bindings are a baseline, not a final commitment. Changes should preserve speed, predictability, and scriptability.

## Product Model

Tasks are structured records. Markdown is supported as task notes, but raw Markdown files are not the primary data model.

This is intentional:

- Structured tasks are easier to sync.
- Field-level conflict resolution is feasible.
- Sorting, deletion tombstones, status, and tags remain reliable.
- Markdown export/import can still be supported.

## Operating Model

- Client and self-hosted server are free to use.
- Donations or sponsorships may support development.
- Official hosted sync service may charge for server resources.
- The product should not lock core functionality behind the official server.
- Optional billing mode is controlled by server configuration and is disabled by default.
- When billing is disabled, every account is treated as Premium and can create projects without limits.
- When billing is enabled, new non-admin accounts default to Free; Free accounts can use Inbox but cannot create additional projects.
- Subscription providers may include Stripe, Creem, and Dodopayments. A successful subscription upgrades the account to Premium.
