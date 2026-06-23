# LemonTodo Mobile App Plan

## Decision

The first mobile client should use React Native with Expo.

React Native is a better fit than Flutter for LemonTodo's first mobile client because the current Web client already uses TypeScript and React. The mobile app can share protocol DTOs, account/session flows, sync client behavior, account-plan limits, and visual language with the Web client while keeping the Rust TUI and server as the protocol authority.

Expo should be used for development and debugging. If native crypto, secure storage, or a future Rust core bridge requires native code, use Expo development builds instead of limiting the app to Expo Go.

Flutter remains a possible future option for a separate client, but it is not the preferred MVP path because it would require maintaining a second Dart implementation of client business logic.

## Goals

- Support Android and iOS from one codebase.
- Match the Terminal-inspired visual language of the TUI and Web client.
- Preserve the existing account password versus master password separation.
- Reuse Web-compatible TypeScript protocol and sync code where practical.
- Keep all Todo content encrypted before it reaches the server.
- Enforce Free and Premium client behavior consistently with the server.

## Non-Goals

- Do not build a general note app.
- Do not add team collaboration or server-side search.
- Do not implement mobile-only sync semantics.
- Do not require a hosted official service.
- Do not move the product away from the documented sync protocol.

## Project Location

The mobile app should live in:

```text
apps/mobile
```

The first implementation can be a TypeScript Expo app. Shared client code that is useful to both Web and mobile should be extracted only when duplication becomes real, for example:

```text
apps/shared
```

or a package under the workspace if the repository later standardizes JavaScript workspaces.

Do not prematurely move Rust core logic behind a mobile native bridge. A Rust bridge can be added later when the TypeScript client path is stable and the app needs stronger sharing with the TUI's local storage, merge, or crypto implementation.

## Login And Unlock Flow

The mobile app should keep authentication and vault unlock as separate steps:

1. Enter server URL.
2. Enter account email and account password.
3. Login to the server and fetch account metadata plus encrypted vault metadata.
4. Enter master password.
5. Decrypt the vault key locally.
6. Pull encrypted sync objects.
7. Decrypt and render projects and tasks locally.

After first setup, the app may remember the server URL, account email, session token, and encrypted vault metadata. It must not persist the plaintext master password. If secure platform storage is added later, storing the decrypted vault key should require an explicit product decision.

Recommended returning-user flow:

1. Open app.
2. If a valid session exists, prompt for master password or approved local unlock method.
3. Sync after unlock.
4. Enter the project list.

## Main Screens

### Server Setup

- Server URL input.
- Basic validation and reachability check against `/v1/server-info`.
- Remember the most recent server URL after a successful login.

### Login Or Register

- Email input.
- Account password input.
- Optional registration entry when the server allows registration.
- Clear error states for invalid credentials, disabled registration, and network failure.

### Unlock

- Master password input.
- Hidden input content.
- Clear failure state when the encrypted vault key cannot be decrypted.

### Project List

- First screen after unlock.
- Projects sorted by the product's current project ordering rules.
- `Inbox` is always present.
- Free accounts only show project navigation and cannot create, rename, archive, or delete projects.
- Premium accounts can create, rename, archive, and delete projects.

### Task List

- Opened by tapping a project.
- Tasks sorted by the product's current task ordering rules.
- Fast add task action only asks for title.
- Task rows support completion toggle and navigation to detail.
- Archive and delete can be exposed through contextual actions.

### Task Detail

- Edit title.
- Edit Markdown note.
- Edit due date.
- Edit tags.
- Archive or delete task.

## Mobile Interaction Rules

- Prefer tap, long press, and swipe interactions for primary mobile workflows.
- Keep hardware keyboard shortcuts as optional enhancements, not required controls.
- Task creation should be fast: title first, details later.
- Avoid modal-heavy flows for common edits.
- Use fixed-height list rows where practical so project and task lists stay scannable.
- Do not expose internal sync debugging views in the first mobile MVP.

## Account Plan Behavior

The app must read account plan metadata from `/v1/account/me`.

When billing is disabled on the server, every account is effectively Premium.

When billing is enabled:

- Free accounts can use Inbox.
- Free accounts cannot create additional projects.
- Free accounts cannot rename, archive, or delete projects.
- Premium accounts can create, rename, archive, and delete projects.

The client should hide or disable unavailable project actions for Free accounts, but the server sync layer remains the final enforcement point.

## Sync Behavior

Mobile sync should follow the same encrypted object protocol as TUI and Web.

Recommended MVP behavior:

- Pull after vault unlock.
- Push after local edits with a short debounce.
- Pull when the app returns to foreground.
- Show compact sync state: syncing, synced, offline, conflict.
- Preserve manual sync as an explicit action.

Do not implement a separate mobile-only background sync model in the first version. Background sync can be considered later after the foreground sync path is reliable on both Android and iOS.

## Visual Direction

The mobile UI should match the Terminal-inspired design already used by the TUI and Web client:

- Compact, scannable list layouts.
- Monospace typography for labels, status, metadata, and command-like elements.
- Restrained green or teal accent color.
- Clear contrast and fixed interaction targets for mobile use.
- No marketing-style landing page inside the app.

The mobile app should feel like the same product as the TUI and Web client, but it should not copy terminal keyboard interaction as the primary phone interface.

## Implementation Phases

### Phase 1: Shell And Auth

- Create Expo app under `apps/mobile`.
- Implement server URL setup.
- Implement login and register flow.
- Implement master password unlock.
- Fetch `/v1/account/me` and vault metadata.

### Phase 2: Local Workspace

- Render project list.
- Render task list.
- Create and edit task title.
- Open task detail.
- Edit note, due date, and tags.

### Phase 3: Sync

- Pull encrypted objects after unlock.
- Push local edits.
- Handle server rejections, including plan limits.
- Show compact sync status.

### Phase 4: Polish

- Add contextual project actions for Premium accounts.
- Add mobile search.
- Add conflict surface.
- Add local unlock improvements if approved.
