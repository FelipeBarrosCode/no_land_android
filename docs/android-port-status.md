# Android port implementation status

## Current checkpoint

The Android SDK/NDK scaffold and first native stream-shell layer are implemented. The Tauri Android project has been generated locally, the Rust library cross-checks for `aarch64-linux-android`, and the maintained Kotlin overlay compiles for the universal debug variant.

This checkpoint does not claim working streaming or a device-tested APK. The Android activity, `SurfaceView`, and `MediaCodec` boundary exist, but Rust/JNI session delivery, Android VPN, secure storage, audio/input, and device validation remain incomplete.

## Implemented

- Dedicated `src-tauri/tauri.android.conf.json` using application identifier `com.noland.connect`, separate from the existing iOS configuration.
- Android-only `android` capability scoped to the `main` window.
- Mobile-safe bundle policy: updater artifacts are disabled, the desktop updater is removed from the merged config, and desktop sidecars are excluded.
- Existing shared remote-agent and contract source resources are retained for provisioning workflows.
- npm commands for Android initialization, generated-project configuration, development, build, and static validation.
- Idempotent `configure:android` step. It overlays maintained files from `src-tauri/android` when that directory exists and fails with an actionable message when `src-tauri/gen/android` has not been generated.
- Ignore rules for generated Android projects, Gradle/build output, local SDK paths, APK/AAB artifacts, keystores, and signing-property files.
- `check:android` validates the scaffold and artifact/security policy using Node.js only; it does not require an Android SDK.
- The Rust build script detects Android explicitly, never stages desktop sidecars for it, and generates static native bindings without attempting the desktop GStreamer build.
- Android selects the existing in-process `noland-ssh` transport instead of desktop `ssh`, `scp`, and PTY executables.
- Android SSH keys are generated in process and stored in the app-private key directory with restrictive Unix permissions; iOS retains its Keychain-backed behavior.
- Desktop clipboard dependencies are excluded from Android. Clipboard calls currently return an explicit native-bridge-required error rather than attempting to use a desktop API.
- Android adaptive packet sizing treats interface MTU as unknown until the Android network bridge can report it, rather than compiling a Linux/macOS implementation.
- Maintained `src-tauri/android` overlay with a private full-screen `StreamActivity`, lifecycle-aware `StreamSurfaceView`, narrow JNI surface callbacks, and a surface-backed `MediaCodecVideoDecoder` access-unit API.
- Android no longer enters desktop Tauri window, native-input, or performance-overlay branches during Rust compilation.
- Local toolchain installed and verified: Android Studio Rabbit 1, API 36, Build Tools 36.0.0, NDK 29.0.14206865, CMake 3.22.1, four Rust Android targets, and Temurin JDK 21.

## Not implemented

- Working native Android Moonlight streaming. The activity/surface/decoder shell exists, but Rust JNI exports, encoded packet delivery, decoder negotiation/recovery, audio output, microphone capture, and input/controller handling are not connected.
- Android VPN/tunnel support, including a `VpnService` implementation, protected sockets, Direct/TURN transitions, and lifecycle reconciliation.
- Android secure storage for application secrets, native clipboard/share/file-picker adapters, notifications, deep links, and other platform integrations.
- Real Android Moonlight runtime replacement for the current link-safe shell ABI adapter. The adapter keeps startup/load safe and returns explicit invalid-state errors for unsupported streaming/input operations.
- Release signing, CI builds, emulator tests, and physical-device acceptance.
- A normal one-command Tauri APK build on this Windows machine until Developer Mode permits Tauri's generated-library symlink, or the build workflow gains a copy fallback.

No Android runtime feature is considered complete because its shared frontend or Rust command already exists.

## Verification

Verified in this checkpoint:

- `npm run check:android` — 52 static scaffold/security checks passed.
- `npm run build` — TypeScript and Vite build passed; the local Node 20.14 runtime is older than Vite's supported 20.19 minimum and emitted a warning.
- `npm run i18n:check` — 1,167 keys across nine locale bundles passed after making the script's project-root resolution Windows-safe.
- `cargo test --locked --manifest-path src-tauri/Cargo.toml -p noland-ssh` — seven transport tests passed.
- `cargo fmt --all --manifest-path src-tauri/Cargo.toml -- --check` — passed.
- `NOLAND_SKIP_NATIVE_BUILD=1 ... cargo check --locked --manifest-path src-tauri/Cargo.toml --lib` — passed with five existing dead-code warnings while desktop bundle artifacts were disabled.
- `cargo check --locked --manifest-path src-tauri/Cargo.toml --lib --target aarch64-linux-android` with the NDK compiler configured — passed.
- `gradlew :app:compileUniversalDebugKotlin -x :app:rustBuildUniversalDebug` — passed; this includes the maintained stream overlay.
- `tauri android build --debug` — frontend and Android Rust library built, then Tauri stopped because Windows symbolic-link creation is disabled.
- Android Rust `cargo build` with the NDK linker — passed; `llvm-readelf` confirmed there are no unresolved `nl_*` symbols after linking the Android shell ABI adapter.
- `gradlew :app:assembleUniversalDebug -x :app:rustBuildUniversalDebug` after copying the already-built ARM64 library — passed and produced `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`.

Run the SDK-free scaffold check from the repository root:

```sh
npm run check:android
```

After installing the required Android toolchain, initialize the generated project with:

```sh
npm run tauri:android:init
```

The initialization command generates `src-tauri/gen/android` and then applies any maintained `src-tauri/android` overlay.
