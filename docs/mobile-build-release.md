# Mobile Testing, Distribution, and Google Play Release

This document is the operational runbook for the LemonTodo React Native and Expo app in `apps/mobile`.

## Build Model

LemonTodo uses native Argon2 to preserve the same authentication and vault KDF parameters as the TUI and Web clients. Expo Go does not include this native module.

- Use the LemonTodo Expo development build for registration, login, encryption, and sync testing.
- Use Expo Go only for UI work that does not require Argon2.
- Rebuild the development client after changing native dependencies or native configuration.
- Use an APK for direct/internal Android distribution.
- Use an Android App Bundle (`.aab`) for Google Play.

## Local Prerequisites

- Node.js and npm
- Android SDK 36 and Build Tools 36
- JDK 17
- Android device with developer mode and USB debugging enabled
- LemonTodo server listening on port `8787`

On the current Linux development machine, the Android npm script uses these fallbacks when environment variables are not already set:

```text
JAVA_HOME=$HOME/.local/share/jdks/temurin-17
ANDROID_HOME=$HOME/Android/Sdk
ANDROID_SDK_ROOT=$ANDROID_HOME
```

## Local Device Testing

Install dependencies and verify the project:

```bash
npm --prefix apps/mobile install
npm --prefix apps/mobile run typecheck
cd apps/mobile && npx expo-doctor
```

Connect and verify the device:

```bash
adb devices -l
```

For a USB-connected device, expose Metro and the local LemonTodo server through ADB:

```bash
adb reverse tcp:8081 tcp:8081
adb reverse tcp:8787 tcp:8787
```

Build, install, and launch the development client:

```bash
npm --prefix apps/mobile run android
```

When using `adb reverse`, enter this server URL in the app:

```text
http://127.0.0.1:8787
```

After the development client is installed, ordinary TypeScript and UI changes only require Metro:

```bash
npm run mobile:start
```

Fast Refresh applies JavaScript changes. Run `npm --prefix apps/mobile run android` again after changing Expo/React Native versions, native dependencies, `app.json` native settings, or Android native code.

Useful diagnostics:

```bash
adb logcat | rg 'LemonTodo|ReactNativeJS'
adb reverse --list
curl http://127.0.0.1:8787/v1/server-info
```

The local debug APK is generated at:

```text
apps/mobile/android/app/build/outputs/apk/debug/app-debug.apk
```

It contains development tooling and is not a production distribution artifact.

## Authentication and Sync Smoke Test

Run this sequence before distributing a build:

1. Connect to `/v1/server-info` from the mobile app.
2. Register a new account and verify that both Argon2 stages complete without blocking navigation.
3. Log out, log in again, and unlock the vault with the same master password.
4. Confirm the account plan is loaded from the server.
5. Confirm Inbox and its sample task match Web/TUI.
6. Create and edit a mobile task, then verify it appears in Web and TUI.
7. Create and edit a task in Web/TUI, then foreground or manually sync mobile and verify it appears.
8. Restart the app and verify session restoration still requires the master password to unlock.
9. Test invalid password, duplicate registration, unavailable server, and request timeout errors.
10. Confirm idle sync does not loop continuously when the workspace is unchanged.

## EAS Setup

The committed `apps/mobile/eas.json` defines these profiles:

- `development`: internal Expo development client
- `preview`: signed APK for testers
- `production`: signed AAB for Google Play

Authenticate and initialize the Expo project:

```bash
cd apps/mobile
npx eas-cli login
npx eas-cli build:configure
```

EAS may add the Expo project ID to `app.json`. Review and commit that change. Prefer EAS-managed Android credentials unless there is an established organization keystore process. Back up any locally managed keystore and passwords; losing the upload key creates release and recovery work.

## Internal APK Distribution

Build a signed tester APK:

```bash
npm --prefix apps/mobile run build:android:preview
```

For a local EAS build on a prepared Linux environment:

```bash
cd apps/mobile
npx eas-cli build --platform android --profile preview --local
```

Install a downloaded/local APK with:

```bash
adb install -r path/to/lemontodo-preview.apk
```

Before sharing it, test installation on a device that does not have the debug build, then repeat registration, login, vault unlock, foreground sync, offline edits, and upgrade testing.

## Production AAB

The Android application ID is `app.lemontodo.mobile`. Treat it as permanent after the first Play upload.

The user-visible version is `expo.version`. Android also requires a monotonically increasing `android.versionCode`. EAS uses remote version management and automatically increments the production build number, but release notes must still identify the intended semantic version.

Build the Play artifact:

```bash
npm --prefix apps/mobile run build:android:production
```

The production profile generates an `.aab`. Do not upload a debug APK to Google Play.

## Google Play Console

Complete these items before requesting review:

1. Create the app in Play Console with the permanent application ID.
2. Enroll in Play App Signing and securely retain the upload-key recovery information.
3. Prepare app title, short/full descriptions, icon, screenshots, phone/tablet declarations, and feature graphic.
4. Publish a privacy policy URL and complete Data Safety accurately.
5. Complete app access instructions, ads declaration, content rating, target audience, and content declarations.
6. Upload the first AAB manually to the Internal testing track.
7. Install from Play, then run the authentication and sync smoke test against a production-like HTTPS server.
8. Promote through closed/open testing as required by the developer account before production rollout.
9. Use a staged production rollout and monitor crashes, ANRs, authentication failures, and sync errors.

After the first Play upload, later releases can use EAS Submit:

```bash
npm --prefix apps/mobile run submit:android
```

Configure the Google service-account key through EAS/CI secrets. Never commit it to Git.

## Play Policy Blockers

Resolve these before production submission:

- Because mobile registration creates an account, provide account-deletion initiation inside the app and an external web deletion page.
- Publish a complete privacy policy covering account metadata, encrypted content, diagnostics, retention, deletion, and self-hosted server behavior.
- If paid Premium is enabled in the Play-distributed app, review Google Play Billing requirements before exposing Stripe, Creem, or DodoPayments purchase links. External payment flows for digital features may be restricted.
- Ensure production endpoints use HTTPS. Do not ship localhost, LAN, test credentials, service-account keys, or debug logging that exposes secrets.
- Verify all third-party licenses and notices, including the GPLv3 client distribution obligations.

## Release Checklist

```text
[ ] Typecheck and Expo Doctor pass
[ ] Preview APK smoke test passes on a clean device
[ ] TUI/Web/mobile interoperability passes
[ ] Production server uses HTTPS and compatible protocol version
[ ] Version and release notes are correct
[ ] EAS production AAB is signed with the expected credentials
[ ] Privacy policy and Data Safety are current
[ ] Account deletion is available in app and on the web
[ ] Billing behavior complies with the selected Play distribution model
[ ] Internal/closed Play test passes
[ ] Rollback and server backup plans are ready
```

Official references:

- [Expo development builds](https://docs.expo.dev/develop/development-builds/create-a-build/)
- [EAS Build setup](https://docs.expo.dev/build/setup/)
- [APK builds with EAS](https://docs.expo.dev/build-reference/apk/)
- [EAS Submit for Android](https://docs.expo.dev/submit/android/)
- [Android App Bundles](https://developer.android.com/studio/publish/upload-bundle)
