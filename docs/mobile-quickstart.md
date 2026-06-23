# Mobile Quickstart

This is the short operational reference for the LemonTodo mobile app in `apps/mobile`.

## Linux Boundaries

- Android local build and USB debugging are supported.
- iOS local native build is not supported on Linux.
- iOS cloud builds through EAS are supported on Linux.

## Install and Verify

```bash
npm --prefix apps/mobile install
npm --prefix apps/mobile run typecheck
cd apps/mobile && npx expo-doctor
```

## Android Local Testing

Start the LemonTodo server locally, then expose Metro and the server to the USB-connected device:

```bash
adb reverse tcp:8081 tcp:8081
adb reverse tcp:8787 tcp:8787
adb devices -l
```

Build and install the Android development client:

```bash
npm --prefix apps/mobile run android
```

After the development client is installed, use Metro for ordinary JS work:

```bash
cd apps/mobile
npm run start
```

Use this server URL inside the app when `adb reverse` is active:

```text
http://127.0.0.1:8787
```

## Android Release Builds

Tester APK:

```bash
npm --prefix apps/mobile run build:android:preview
```

Google Play AAB:

```bash
npm --prefix apps/mobile run build:android:production
```

Submit to Google Play:

```bash
npm --prefix apps/mobile run submit:android
```

## iOS Builds from Linux

Development build:

```bash
npm --prefix apps/mobile run build:ios:development
```

Internal tester build:

```bash
npm --prefix apps/mobile run build:ios:preview
```

Production App Store / TestFlight build:

```bash
npm --prefix apps/mobile run build:ios:production
```

Submit to App Store Connect:

```bash
npm --prefix apps/mobile run submit:ios
```

## iPhone Testing from Linux

Development build on your own iPhone:

```bash
npm --prefix apps/mobile run build:ios:development
cd apps/mobile
npm run start
```

Then install the development build from the EAS build page and open it on the iPhone.

For the LemonTodo server URL inside the app, use a reachable LAN address, tunnel URL, or public HTTPS URL. Do not use `http://127.0.0.1:8787` on iPhone.

Internal tester install:

```bash
cd apps/mobile
npx eas-cli device:create
npm run build:ios:preview
```

Then send the EAS internal distribution link to the tester's iPhone.

Production-like testing:

```bash
npm --prefix apps/mobile run build:ios:production
npm --prefix apps/mobile run submit:ios
```

Then install the build through TestFlight on the iPhone.

## Local iOS Testing on macOS Only

```bash
npm --prefix apps/mobile run ios
```

This requires macOS, Xcode, and either an iOS Simulator or a connected iPhone.

## Smoke Test

Run this before shipping any tester or production build:

1. Connect to the server.
2. Register or log in.
3. Unlock the vault with the master password.
4. Confirm the server-reported plan loads correctly.
5. Confirm Inbox and sample data match Web and TUI.
6. Create and edit a task on mobile, then verify sync to Web and TUI.
7. Create and edit a task in Web or TUI, then verify sync to mobile.
8. Restart the app and confirm session restoration still requires vault unlock.

## Full Runbook

For full Android, iOS, TestFlight, and store-release details, use:

- [mobile-build-release.md](/home/lin/Workspace/LemonTodo/docs/mobile-build-release.md)
