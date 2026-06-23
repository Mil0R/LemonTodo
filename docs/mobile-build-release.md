# Mobile Testing, Distribution, and Store Release

This document is the operational runbook for the LemonTodo React Native and Expo app in `apps/mobile`.

## Build Model

LemonTodo uses native Argon2 to preserve the same authentication and vault KDF parameters as the TUI and Web clients. Expo Go does not include this native module.

- Use the LemonTodo Expo development build for registration, login, encryption, and sync testing.
- Use Expo Go only for UI work that does not require Argon2.
- Rebuild the development client after changing native dependencies or native configuration.
- Use an APK for direct/internal Android distribution.
- Use an Android App Bundle (`.aab`) for Google Play.
- Use an EAS-managed iOS build for TestFlight and App Store distribution.

## Platform Boundaries

On the current Linux development machine:

- Android local build, install, USB debugging, and log capture are supported.
- iOS local native build is not supported because Xcode and the iOS Simulator require macOS.
- Shared React Native and Expo code can still be developed on Linux.
- iOS release artifacts can still be produced with EAS cloud builds from Linux.

Practical meaning:

- `npm --prefix apps/mobile run android` works on Linux when the Android toolchain is installed.
- `npm --prefix apps/mobile run ios` requires macOS with Xcode and an iOS Simulator or connected iPhone.
- `npx eas-cli build --platform ios ...` can be started from Linux because the native build runs on Expo's macOS workers.

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
cd apps/mobile
npm run start
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

## Local iOS Testing

Linux cannot perform local iOS native compilation or simulator testing for this project.

To do local iOS development you need:

- a macOS machine
- Xcode
- CocoaPods
- an iOS Simulator or a physical iPhone

On macOS, the local iOS flow is:

```bash
npm --prefix apps/mobile install
npm --prefix apps/mobile run typecheck
cd apps/mobile && npx expo-doctor
npm --prefix apps/mobile run ios
```

For a physical iPhone on the same LAN, use the machine's reachable server URL instead of `127.0.0.1`, or expose the server through a tunnel or reverse proxy.

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

The same EAS project can also build iOS artifacts. For iOS, use:

- development build for device debugging
- internal distribution for ad hoc tester installs
- App Store distribution for TestFlight and production release

Authenticate and initialize the Expo project:

```bash
cd apps/mobile
npx eas-cli login
npx eas-cli build:configure
```

EAS may add the Expo project ID to `app.json`. Review and commit that change. Prefer EAS-managed Android credentials unless there is an established organization keystore process. Back up any locally managed keystore and passwords; losing the upload key creates release and recovery work.

For iOS, prefer EAS-managed credentials unless the team already controls certificates and provisioning profiles centrally. Apple signing assets are operationally sensitive in the same way as Android upload keys.

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

## iOS Development and Internal Distribution

From Linux, use EAS cloud builds for iOS:

```bash
cd apps/mobile
npx eas-cli build --platform ios --profile development
```

Use this when you need an iOS development client for device-side testing with native modules.

For a tester-facing internal iOS build:

```bash
cd apps/mobile
npx eas-cli build --platform ios --profile preview
```

Recommended profile intent:

- `development`: development client, debugger-friendly
- `preview`: internal distribution for testers
- `production`: App Store / TestFlight binary

Operational notes:

- The Apple Developer account must be configured in Expo/EAS.
- Test devices may need to be registered for ad hoc installs if you do not use TestFlight.
- Native module changes still require a new iOS build, just like Android development builds require reinstall after native changes.
- Local JavaScript changes continue to use Metro once the development build is installed on the device.

## Testing iPhone Builds from Linux

Use one of these paths depending on the test goal.

### Path A: Development build on a personal iPhone

Use this when you want native-module support plus live JavaScript iteration.

1. Build the development client:

```bash
npm --prefix apps/mobile run build:ios:development
```

2. Open the finished build page in the EAS dashboard and install the build on the iPhone.
3. Start Metro on the Linux machine:

```bash
cd apps/mobile
npm run start
```

4. Launch the LemonTodo development build on the iPhone and connect it to Metro.
5. In the app, do not use `http://127.0.0.1:8787` for the LemonTodo server unless the server is running on the phone itself.
6. Instead, use one of:
   - the Linux machine's LAN address, for example `http://192.168.x.x:8787`
   - a reverse proxy or tunnel URL
   - a public HTTPS server

Notes:

- Real iPhone device builds on EAS require a paid Apple Developer account.
- The iPhone must be able to reach both Metro and the LemonTodo server over the network.
- Unlike Android USB testing, there is no `adb reverse` equivalent for iPhone in this flow.

### Path B: Internal iPhone install for testers

Use this when you want a self-contained build that does not depend on Metro.

1. Register the tester device if ad hoc provisioning is used:

```bash
cd apps/mobile
npx eas-cli device:create
```

2. Build the internal iPhone package:

```bash
npm --prefix apps/mobile run build:ios:preview
```

3. Send the internal distribution URL from the EAS build page to the tester.
4. Open the link on the iPhone and install the build.
5. Point the app at a reachable server URL, normally a LAN address or public HTTPS endpoint.

Important:

- If you add a new iPhone after the ad hoc provisioning profile was created, create a new build so the new device is included.
- Internal distribution is best for install and feature validation, not for fast JS iteration.

### Path C: TestFlight

Use this when you want the most production-like iPhone test path.

1. Build the production iOS binary:

```bash
npm --prefix apps/mobile run build:ios:production
```

2. Submit it to App Store Connect:

```bash
npm --prefix apps/mobile run submit:ios
```

3. Wait for App Store Connect processing to finish.
4. Add testers in TestFlight.
5. Install TestFlight on the iPhone, accept the invite, and install LemonTodo.
6. Run the full authentication and sync smoke test against an HTTPS server.

## Production AAB

The Android application ID is `app.lemontodo.mobile`. Treat it as permanent after the first Play upload.

The user-visible version is `expo.version`. Android also requires a monotonically increasing `android.versionCode`. EAS uses remote version management and automatically increments the production build number, but release notes must still identify the intended semantic version.

Build the Play artifact:

```bash
npm --prefix apps/mobile run build:android:production
```

The production profile generates an `.aab`. Do not upload a debug APK to Google Play.

## Production iOS Build

Build the App Store binary with EAS:

```bash
cd apps/mobile
npx eas-cli build --platform ios --profile production
```

The result is an App Store submission artifact handled through EAS and Apple tooling. Treat the iOS bundle identifier as permanent after the first App Store Connect release, the same way the Android application ID becomes permanent after the first Play upload.

Before the first production iOS build:

1. Create the app in App Store Connect.
2. Decide the permanent iOS bundle identifier.
3. Confirm the Apple Team, signing model, and app capabilities.
4. Prepare the privacy policy, screenshots, app description, support URL, and age/content declarations.

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

## TestFlight and App Store Connect

Use TestFlight before any App Store production release.

First-time setup:

1. Create the app record in App Store Connect with the permanent bundle identifier.
2. Complete app metadata, privacy policy URL, support URL, and age rating questionnaire.
3. Prepare iPhone screenshots and any required iPad screenshots if the app declares iPad support.
4. Configure export compliance and encryption declarations accurately. LemonTodo uses encryption, so do not skip this review.
5. Ensure account deletion handling and privacy disclosures match the production server behavior.

Upload flow:

```bash
cd apps/mobile
npx eas-cli submit --platform ios --profile production
```

Or submit the completed iOS build manually from the EAS dashboard/App Store Connect flow if preferred.

Recommended release path:

1. Ship a production-profile iOS build to TestFlight.
2. Install from TestFlight and repeat the authentication and sync smoke test against an HTTPS server.
3. Validate upgrade behavior from previous builds.
4. Check crash reports, login failures, sync behavior, and any App Store review notes.
5. Promote the same or a follow-up build to App Store review.

## Apple Policy Blockers

Resolve these before App Store submission:

- Account creation requires account deletion initiation in app and a working deletion path.
- Privacy disclosures must accurately describe account metadata, encrypted content, diagnostics, retention, and self-hosted behavior.
- Export compliance answers must match the app's encryption use.
- If paid Premium is enabled in the iOS-distributed app, review Apple's in-app purchase rules before exposing external purchase links for digital entitlements.
- Production iOS builds must target HTTPS endpoints. Do not ship local URLs, LAN-only assumptions, debug credentials, or verbose logs that may expose secrets.

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
[ ] iOS development or preview build smoke test passes on at least one iPhone
[ ] TUI/Web/mobile interoperability passes
[ ] Production server uses HTTPS and compatible protocol version
[ ] Version and release notes are correct
[ ] EAS production AAB is signed with the expected credentials
[ ] EAS production iOS build completes with the expected Apple team and bundle identifier
[ ] Privacy policy and Data Safety are current
[ ] Account deletion is available in app and on the web
[ ] Billing behavior complies with the selected Play distribution model
[ ] Internal/closed Play test passes
[ ] TestFlight test passes
[ ] Rollback and server backup plans are ready
```

Official references:

- [Expo development builds](https://docs.expo.dev/develop/development-builds/create-a-build/)
- [EAS Build setup](https://docs.expo.dev/build/setup/)
- [APK builds with EAS](https://docs.expo.dev/build-reference/apk/)
- [EAS Submit for Android](https://docs.expo.dev/submit/android/)
- [EAS iOS build guide](https://docs.expo.dev/build-reference/ios-builds/)
- [EAS Submit for iOS](https://docs.expo.dev/submit/ios/)
- [Android App Bundles](https://developer.android.com/studio/publish/upload-bundle)
- [TestFlight overview](https://developer.apple.com/testflight/)
- [App Store Connect](https://developer.apple.com/app-store-connect/)
