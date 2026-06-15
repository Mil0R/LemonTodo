# LemonTodo Product MVP

## Positioning

LemonTodo is not a general notes app, Notion replacement, Joplin clone, or team project-management system.

The MVP is:

- A developer-first Todo List.
- TUI-first.
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
- Clear sync status: pending, synced, offline, conflict.
- Conflict view with explicit user resolution.

## Explicit Non-Goals For MVP

- Rich text editing.
- Attachments.
- Calendar UI.
- Complex recurring tasks.
- Team collaboration.
- Server-side search.
- Web app.
- Plugin system.
- CRDT-based real-time collaboration.
- Markdown vault as the only source of truth.

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

