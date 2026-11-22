# Mobile Remote Controller (`apps/mobile`)

CheapRouter Harness includes a peer-to-peer Flutter mobile remote controller
backed by the Go Client and Link runtime via a C ABI (`cmd/sailry-mobile` ->
`apps/mobile/packages/sailry_bridge`).

The mobile app has **no account or cloud login** (no CheapRouter login, no
Sailry cloud login, no subscription/billing UI, and no cloud update check).
It connects directly to paired Hosts via iroh pairing tickets or 6-digit
pairing codes to browse sessions, send messages, inspect execution timelines,
handle approvals/questions, and open remote terminals and workspaces.

## Identifiers

- **Display name**: `CheapRouter Harness`
- **Android `applicationId` / `namespace`**: `cc.cheaprouter.harness`
- **iOS `PRODUCT_BUNDLE_IDENTIFIER`**: `cc.cheaprouter.harness`
- **Icon source**: `assets/branding/cheaprouter.png` (1024x1024 RGBA)

## Prerequisites

- **Go**: `1.27.3` (`unset GOROOT && export GOTOOLCHAIN=go1.27.3`)
- **Flutter**: version pinned in `apps/mobile/.flutter-version` (`3.44.7`)
- **Android**: JDK 17, Android SDK `platforms;android-36`,
  `build-tools;36.0.0`, Android NDK `28.2.13676358`
- **iOS**: macOS with Xcode, CocoaPods, and an Apple Developer Team /
  provisioning profile for `cc.cheaprouter.harness` (for physical devices)

## Android Builds

Gradle (`apps/mobile/android/app/build.gradle.kts`) automatically invokes
`cmd/sailry-mobile/build.sh android <abi>` during `preBuild` to compile
`libsailry_mobile.so` into `.runtime/native/android-mobile-jni/<abi>/`.

### Debug APK (arm64-v8a)

```sh
cd apps/mobile
flutter pub get
flutter build apk --debug --target-platform android-arm64
```

Output: `apps/mobile/build/app/outputs/flutter-apk/app-debug.apk`.

### Release APK (arm64-v8a)

Local unsigned/debug-signed release build:

```sh
cd apps/mobile
flutter pub get
flutter build apk --release --target-platform android-arm64
```

Distribution-signed release build (all four keystore variables are required
together; set `SAILRY_ANDROID_RELEASE=1` to enforce signing):

```sh
export SAILRY_ANDROID_RELEASE=1
export ANDROID_KEYSTORE_PATH=/path/to/release.jks
export ANDROID_STORE_PASSWORD=...
export ANDROID_KEY_ALIAS=...
export ANDROID_KEY_PASSWORD=...

cd apps/mobile
flutter pub get
flutter build apk --release --target-platform android-arm64
```

Output: `apps/mobile/build/app/outputs/flutter-apk/app-release.apk`.

### Building the Go C ABI library standalone

```sh
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/28.2.13676358"
sh cmd/sailry-mobile/build.sh android arm64-v8a
```

Supported ABIs: `arm64-v8a`, `armeabi-v7a`, `x86_64`.

## iOS Builds (macOS Only)

iOS builds require macOS with Xcode and CocoaPods. The Xcode project
(`apps/mobile/ios/Runner.xcodeproj`) has a build phase (`Build Sailry Native`)
that runs `apps/mobile/ios/build_native.sh` -> `cmd/sailry-mobile/build.sh ios`
to build and `lipo` `libsailry_mobile.a` into `$(BUILT_PRODUCTS_DIR)/SailryNative`.

### Simulator Build

```sh
cd apps/mobile
flutter pub get
flutter build ios --simulator --debug
```

### Device / Archive Build

1. Open `apps/mobile/ios/Runner.xcworkspace` in Xcode.
2. Select the `Runner` target -> **Signing & Capabilities**.
3. Choose your Apple Developer Team and verify Bundle Identifier
   `cc.cheaprouter.harness` (and provisioning profile).
4. Build from Xcode or via Flutter:

```sh
cd apps/mobile
flutter pub get
flutter build ipa --release
```

## Verification

Go core & mobile bridge:

```sh
unset GOROOT
export GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0
go build ./...
go vet ./...
go test ./internal/client/... ./internal/mobile/...

# Host CGO shared library sanity check
CGO_ENABLED=1 go build -buildmode=c-shared -o /tmp/libsailry_mobile.so ./cmd/sailry-mobile
```

Flutter analysis & widget/unit tests:

```sh
cd apps/mobile
flutter pub get
flutter analyze
flutter test
```

## CI

Manual Android debug builds are available via GitHub Actions workflow
`.github/workflows/mobile-android-debug.yml` (`workflow_dispatch` only; does
not block desktop test or release pipelines).
