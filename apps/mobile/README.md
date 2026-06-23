# LemonTodo Mobile

React Native and Expo client for Android and iOS.

See [`docs/mobile-build-release.md`](../../docs/mobile-build-release.md) for device testing, APK distribution, and Google Play release procedures.

## Run

Real registration and login use a native Argon2 module so the KDF stays compatible with TUI and Web. Expo Go does not contain that module; use an Expo development build:

```bash
npm --prefix apps/mobile install
npm --prefix apps/mobile run android
```

After the development build is installed, start Metro for subsequent sessions:

```bash
npm --prefix apps/mobile run start
```

`npm --prefix apps/mobile run expo-go` remains available for UI-only work that does not require registration or vault unlock. Use `npm --prefix apps/mobile run ios` for a local iOS development build and `npm --prefix apps/mobile run web` for Web.

Typecheck:

```bash
npm --prefix apps/mobile run typecheck
```

## Local Sync Smoke Test

Expo Web runs on a different origin than the local server, so start the server with:

```bash
LEMONTODO_SERVER_HOST=127.0.0.1 \
LEMONTODO_SERVER_PORT=8787 \
LEMONTODO_ALLOW_REGISTRATION=true \
LEMONTODO_CORS_ALLOW_ORIGIN=http://localhost:8082 \
cargo run -p lemontodo-server
```

Create a test account from TUI or Web, then log in from the mobile app with the same server URL, email, and master password. The mobile app should pull existing projects/tasks after unlock and push local edits automatically while unlocked.

## Current Scope

This first slice is a mobile UI shell for the agreed workflow:

- server URL
- account login and registration when allowed by the server
- master password unlock
- project list
- task list
- task detail
- Free and Premium project-management behavior

Server discovery, registration, login, logout, account status fetch, encrypted vault metadata fetch, local vault-key unlock, local workspace persistence, session restore, manual encrypted sync, and session-scoped automatic sync are wired. Registration follows the login flow: server URL first, then email, master password, and master password confirmation.

Automatic sync runs silently after local edits, at a fixed interval while the app is unlocked, and when the app returns to the foreground.

## Archive And Restore

- Archiving a project or task requires confirmation.
- Archived projects appear under `ARCHIVED PROJECTS` at the bottom of the project list.
- Archived tasks appear under `ARCHIVED TASKS` at the bottom of their project task list.
- Select `RESTORE` to return a project to the active list. Restored tasks return with `open` status.
- Inbox cannot be archived, and Free accounts cannot archive projects.
- Sync protocol v1 has no project-archive field. Mobile keeps project archive state locally while retaining the encrypted project and tasks remotely, so it does not propagate that state to TUI or Web and does not delete the remote data.

The current server auth implementation uses a master-password derived auth hash for login, matching the existing Web and TUI clients. If account password and master password become separate credentials later, the mobile login flow should split those fields at the protocol layer.
