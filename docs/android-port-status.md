# Android port implementation status

## Current checkpoint

As of 2026-10-21, native H.264 Moonlight streaming works on the USB-connected ARM64 Pixel 7a. The app links the real `noland-moonlight/0.3.0` runtime, presents a private `StreamActivity`, decodes through NDK `AMediaCodec` into a `SurfaceView`, and plays Opus audio through a bounded OpenSL ES queue. Back disconnects and returns to the management UI; a subsequent Play starts another session on the existing instance.

WireGuard uses Android `VpnService` with native consent and persisted configuration. App-private state is preserved across in-place APK updates. This is a functional physical-device checkpoint, not acceptance of every Android API level, codec, input device, or background/network transition. The older `docs/android-emulator-compatibility.md` remains a historical emulator audit.

## Implemented

- Dedicated `src-tauri/tauri.android.conf.json` using application identifier `com.noland.connect`, separate from the existing iOS configuration.
- Android-only `android` capability scoped to the `main` window.
- Mobile-safe bundle policy: updater artifacts are disabled, the desktop updater is removed from the merged config, and desktop sidecars are excluded.
- State-agent, network-agent/contracts, and microphone-receiver sources are embedded as compact archives and extracted lazily for remote provisioning; Android does not duplicate them as directory-style Tauri resources.
- npm commands for Android initialization, generated-project configuration, development, build, and static validation.
- Idempotent `configure:android` step. It overlays maintained files from `src-tauri/android` when that directory exists and fails with an actionable message when `src-tauri/gen/android` has not been generated.
- Ignore rules for generated Android projects, Gradle/build output, local SDK paths, APK/AAB artifacts, keystores, and signing-property files.
- `check:android` validates the scaffold and artifact/security policy using Node.js only; it does not require an Android SDK.
- The Rust build script detects Android explicitly, never stages desktop sidecars for it, and generates static native bindings without attempting the desktop GStreamer build.
- Android selects the existing in-process `noland-ssh` transport instead of desktop `ssh`, `scp`, and PTY executables.
- Android SSH keys are generated in process and stored in the app-private key directory with restrictive Unix permissions; iOS retains its Keychain-backed behavior.
- Desktop clipboard dependencies are excluded from Android. Clipboard calls currently return an explicit native-bridge-required error rather than attempting to use a desktop API.
- Android adaptive packet sizing treats interface MTU as unknown until the Android network bridge can report it, rather than compiling a Linux/macOS implementation.
- Maintained `src-tauri/android` overlay with private full-screen `StreamActivity`, aspect-fit `StreamSurfaceView`, token/incarnation-safe JNI callbacks, and the iOS-reference [in-stream drawer, touch modes, keyboard, and controller controls](android-stream-controls.md).
- Return to Noland preserves the runtime; Android dashboard Return to stream/Disconnect actions support reentry without a fresh stream launch. Native statistics follow the existing per-instance preference.
- Physical gamepad/mouse adapters, virtual/physical player-one merging, persistent controller presence, and ordered text/key input are connected; physical-device peripherals still need acceptance.
- Rust owns acquired `ANativeWindow` references until runtime-actor detach acknowledgement. Initial surface wait is bounded; surface recovery and disconnect are serialized with startup.
- Android CMake source-builds the pinned Opus/Mbed TLS dependencies and excludes desktop SDL/GStreamer. The legacy shell ABI adapter is no longer linked.
- Native video uses bounded Moonlight pull-frame ownership and IDR recovery. Initial negotiation advertises H.264 only; HEVC support in the renderer is not advertised until device capability probing is implemented.
- Native audio decodes Opus into bounded OpenSL ES playback buffers; microphone forwarding is a separate, still-incomplete path.
- Native WireGuard VPN consent, startup reconnect, healthy handshake, and app-private state restoration have been verified on the physical device.
- Android no longer enters desktop Tauri window, native-input, or performance-overlay branches during Rust compilation.
- Local toolchain installed and verified: Android Studio Rabbit 1, API 36, Build Tools 36.0.0, NDK 29.0.14206865, CMake 3.22.1, four Rust Android targets, and Temurin JDK 21.
- Android app secrets persist in an AES-256-GCM app-private vault and are scrubbed from `state.json`; hardware-backed Android Keystore wrapping remains future hardening.
- GitHub Actions builds and uploads separate ARM64 and x86_64 debug APKs.
- The x86_64 APK has been installed and cold-started on an API 36 emulator; WebView UI, Rust startup, network access, and encrypted state read-back were verified.

## Remaining implementation / acceptance

- Android microphone capture/sender: the existing UI can still report a missing `noland-mic-sender` sidecar. This does not prevent video or playback audio.
- Runtime HEVC size/profile capability probing, HDR/10-bit/AV1 acceptance, physical controller LED/battery/motion/rumble output, and user-facing automatic relative mouse capture policy.
- Hardware-backed Android Keystore wrapping, native clipboard/share/file-picker adapters, complete notification permission handling, and deep links.
- Broader surface recreation, background/foreground, process death, Wi-Fi/cellular, Direct/TURN, Bluetooth/USB audio-route, and OEM lifecycle acceptance.
- Release signing, automated emulator/instrumentation CI, API-24 runtime coverage, and physical devices beyond the tested Pixel 7a.
- Windows Tauri builds still require Developer Mode for generated-library symlinks or a copy fallback.

No Android runtime feature is considered complete because its shared frontend or Rust command already exists.

## Verification

2026-10-21 physical-device checkpoint:

- `npm run check:android`: SDK-free scaffold/security/native-integration checks passed, including maintained control files, JNI declarations, and Android dashboard reentry.
- `npm run build`: TypeScript/Vite passed (existing large-chunk warning).
- ARM64 native CMake build and no-undefined shared-library link passed; native API-24 source compilation also covered ARMv7 and x86_64.
- Full `tauri android build --debug --target aarch64 --apk --config src-tauri/tauri.android.conf.json` passed after freeing disk space, including Rust/native linking, Kotlin compilation, and APK packaging. Earlier low-space runs required a local cdylib-only workaround.
- APK installed in place; state and VPN configuration survived.
- Real encrypted RTSP handshake, hardware H.264 decoder, native stream Activity, and audio packets were observed. One stream reached 2,065 received/rendered frames with zero drops before Back; another Play reconnected successfully. These are smoke-test observations, not sustained performance guarantees.
- Back returned the runtime to Idle and stopped microphone runtime ownership without deleting the remote instance.
- Persisted Rust lifecycle/input regressions cover stale routes/incarnations, cleanup ownership, dashboard reentry, controller arrival/retry, and ordered keyboard operations; focused standalone harnesses exercise production logic.
- Kotlin gesture, IME bookkeeping, and hardware-input suites passed; native video recovery passed ASan/UBSan checks. See the stream controls document for commands.
- On-phone checks verified the three-finger keyboard shortcut with controller mode both off and on, Done, persisted touch/controller preferences, native statistics, and dashboard reentry after waiting longer than the surface-loss timeout.

Historical scaffold/emulator checks (before native stream integration):

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

The initialization command generates `src-tauri/gen/android` and then applies the maintained `src-tauri/android` overlay. Re-run `npm run configure:android` after changing Kotlin/manifest overlay files before packaging an existing generated project.

For a disk-conscious local ARM64 APK build:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 npx tauri android build \
  --debug --target aarch64 --apk --config src-tauri/tauri.android.conf.json
```

Use a Gradle-compatible JDK (the local build used JDK 24; Android Studio's bundled JDK 25 was incompatible with this Gradle/Groovy stack). Leave several GB free for native/Rust intermediates and packaging.
