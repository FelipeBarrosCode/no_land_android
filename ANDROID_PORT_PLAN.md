# No Land Android Port Plan

## 1. Purpose

Create a new Android repository from a copy of this repository and deliver an Android client for No Land with the same core workflows as the desktop and iOS/mobile implementations.

The Android port will use:

- this repository as the product, UI, Rust, orchestration, and protocol source of truth;
- `moonlight-stream/moonlight-android` as the reference for Android GameStream integration, hardware video decoding, audio output, touch/controller input, JNI boundaries, Android lifecycle handling, and native build layout;
- Android-native replacements for the Apple-specific implementations currently present in this repository.

Reference repository:

- <https://github.com/moonlight-stream/moonlight-android>

This is a plan for a new repository. It does not copy Moonlight Android's application wholesale and does not replace No Land's existing product workflows with Moonlight's UI, database, or account model.

## 2. Starting point

The new repository should initially be created from the latest accepted commit of:

- `FelipeBarrosCode/no_land_mobile_port`

The current mobile repository already contains important Android-reusable work:

- responsive React/TypeScript screens;
- a Tauri mobile entry point;
- shared typed frontend/Rust command contracts;
- pure-Rust SSH support;
- secure-state abstractions;
- Moonlight pairing and GameStream domain code;
- the vendored `moonlight-common-c` native dependency;
- network, provisioning, storage, and remote-management workflows.

The Android repository should not be created by copying only `src/`. The copy must include the Rust workspace, native Moonlight dependency, shared agents, contracts, scripts, documentation, and tests.

### Repository bootstrap

The implementation repository should be created separately from the source repository:

```sh
git clone https://github.com/FelipeBarrosCode/no_land_mobile_port.git no_land_android
cd no_land_android
git remote rename origin upstream-mobile
git remote add origin <new-noland-android-repository-url>
```

The first Android commit should record the source revision used for the fork. Do not delete the desktop or iOS code immediately; isolate platform-specific code first and remove only after Android and existing desktop behavior have replacement coverage.

## 3. Goals

1. Ship a No Land Android application using Tauri 2, React, TypeScript, Rust, Kotlin/Java, and the Android NDK.
2. Preserve the current No Land cloud-management and remote-provisioning workflows.
3. Preserve the existing TypeScript/Rust command names and serialized contracts where practical.
4. Provide an embedded, full-screen native Moonlight stream surface inside the No Land product flow.
5. Use Android hardware decoding through `MediaCodec` for H.264, HEVC, and supported future codecs.
6. Preserve touch, mouse, keyboard, game-controller, virtual-controller, audio, microphone, clipboard, reconnect, and diagnostics behavior.
7. Replace desktop process execution, iOS Network Extension, Apple secure storage, and Apple media APIs with Android implementations.
8. Keep Android code behind explicit platform adapters so desktop and iOS behavior remains understandable.
9. Validate behavior on physical Android phones, tablets, Android TV/large-screen devices where supported, and multiple GPU/vendor combinations.

## 4. Non-goals

- Rebuilding the complete Moonlight Android application.
- Copying Moonlight Android's home screen, preferences, computer database, discovery UI, or account model.
- Replacing No Land's Vast.ai, provisioning, storage, networking, or settings workflows with Moonlight equivalents.
- Assuming a WebView-only stream surface is sufficient for gaming.
- Shipping a first version that silently disables VPN, streaming, microphone, terminal, upload, or recovery behavior.
- Adding Android support to the default desktop build command.
- Committing generated Gradle/build output, signing keys, keystores, APKs, AABs, local SDK paths, or device-specific state.

## 5. Reference findings from Moonlight Android

Moonlight Android is a mature Android GameStream client and should be used as an implementation reference for native streaming. The current upstream structure demonstrates several relevant boundaries:

| Area | Moonlight Android reference | No Land Android application |
| --- | --- | --- |
| Android packaging | Gradle Android application, Java 17, NDK build | Tauri-generated Android shell plus source-controlled native Android module/templates |
| Native core | `app/src/main/jni/moonlight-core`, `moonlight-common-c`, Opus, OpenSSL | Reuse the existing No Land `src-tauri/native/moonlight-common-c` where compatible; keep one pinned revision |
| Video | `MediaCodecDecoderRenderer` and `MediaCodecHelper` | Android `MediaCodec` adapter receiving frames/configuration from the No Land stream session |
| Audio | `AndroidAudioRenderer` | Android audio renderer plus microphone capture adapter for No Land host-agent forwarding |
| JNI | `MoonBridge` and `simplejni.c` | Narrow Rust/Kotlin/JNI bridge; do not expose the entire No Land state model to Java |
| Input | touch contexts, keyboard translation, controller handling, virtual controller | Android native input adapter translating to the existing No Land input command model |
| Stream activity | `Game` and `StreamView` | No Land native stream activity/view presented as the embedded stream route |
| Discovery | mDNS/NSD services | Keep No Land's host/provisioning path; use Android NSD only where local discovery is explicitly required |
| Power/lifecycle | wake lock and activity lifecycle | Android lifecycle adapter, stream foreground service policy, wake lock while streaming |

The reference's current build configuration uses Android Gradle tooling, Java 17, an Android NDK, a minimum API level, and native libraries. Exact versions must be selected during the Android build spike rather than blindly copied from upstream.

### Reference files to study during implementation

- `app/build.gradle`
- `app/src/main/AndroidManifest.xml`
- `app/src/main/java/com/limelight/Game.java`
- `app/src/main/java/com/limelight/ui/StreamView.java`
- `app/src/main/java/com/limelight/binding/video/MediaCodecDecoderRenderer.java`
- `app/src/main/java/com/limelight/binding/video/MediaCodecHelper.java`
- `app/src/main/java/com/limelight/binding/audio/AndroidAudioRenderer.java`
- `app/src/main/java/com/limelight/nvstream/jni/MoonBridge.java`
- `app/src/main/java/com/limelight/binding/input/ControllerHandler.java`
- `app/src/main/java/com/limelight/binding/input/touch/AbsoluteTouchContext.java`
- `app/src/main/java/com/limelight/binding/input/touch/RelativeTouchContext.java`
- `app/src/main/jni/moonlight-core/Android.mk`
- `app/src/main/jni/moonlight-core/simplejni.c`

These files are references for behavior and boundaries. Any source copied into the new project must retain the applicable GPL notices, attribution, and license obligations, and should be kept in a clearly isolated adapter or dependency directory.

## 6. Target architecture

```text
React management UI
  -> Zustand action
  -> typed src/lib/backend.ts function
  -> Tauri invoke/event
  -> Rust command/service
  -> Android platform adapter
  -> Kotlin/Java/JNI/native stream layer
  -> MediaCodec / AudioTrack / Android input APIs
  -> remote GameStream host
```

### 6.1 Shared product layer

Keep these layers shared with desktop and the existing mobile implementation:

- `src/` screens, components, localization, and store logic, with Android layout fixes;
- `src/lib/backend.ts` typed command facade;
- `src/lib/types.ts` and serialized DTOs;
- provisioning, Vast.ai, instance reconciliation, and remote-agent protocols;
- Rust domain models and services that do not require local operating-system facilities;
- Moonlight pairing, host identity, application launch, protocol configuration, metrics, and session state;
- network connection-manager state machine and Direct/TURN policy;
- shared-storage business rules and upload/export state machines;
- test fixtures and contract tests.

### 6.2 Android platform layer

Add explicit Android implementations under a structure similar to:

```text
src-tauri/src/platform/
  mod.rs
  desktop.rs
  ios.rs                  # existing/reference implementation where applicable
  android.rs              # Android capability and lifecycle adapter

src-tauri/src/moonlight/platform/
  mod.rs
  desktop_input.rs
  android.rs              # Android stream/input/surface boundary
  ...

src-tauri/android/         # source-controlled Android templates/custom code
  app/
    src/main/java/com/noland/connect/
    src/main/cpp/
    src/main/AndroidManifest.xml
    build.gradle
  gradle/
  settings.gradle

src-tauri/gen/android/     # generated Tauri project; ignored where possible
```

The exact generated directory layout must follow the Tauri CLI version used by the new repository. Source-controlled Android code must not depend on hand-edited generated files without a deterministic regeneration/patch step.

### 6.3 Capability model

Extend the existing capability model with Android-specific values rather than inferring support from the browser user agent:

```ts
interface ClientCapabilities {
  platform: "android" | "ios" | "desktop";
  embeddedStreaming: boolean;
  managedVpn: boolean;
  remoteTerminal: boolean;
  filePicker: boolean;
  nativeShare: boolean;
  selfUpdate: boolean;
  nativeClipboard: boolean;
  hardwareVideoDecoder: boolean;
  microphoneCapture: boolean;
  gameControllerInput: boolean;
}
```

The UI may explain unavailable runtime capabilities, but capability flags must not be used to hide required product workflows permanently.

## 7. Embedded Moonlight integration

### 7.1 Recommended presentation model

Use one Android-native stream presentation route, launched from the Tauri application and owned by the Android layer. It should be full-screen from the user's perspective and return to the No Land dashboard without opening a desktop-style second Tauri window.

Preferred initial design:

1. React requests stream start through the existing typed backend facade.
2. Rust prepares the No Land session, host, application, stream configuration, and transport.
3. Rust asks the Android adapter to create/present a native stream container.
4. The Android layer presents a dedicated `StreamActivity` or an equivalent native view container.
5. The stream view owns the `Surface`, decoder, audio renderer, input dispatcher, controls, and lifecycle callbacks.
6. Android sends native stream events back through the Tauri/Rust bridge.
7. Rust remains the owner of No Land session state and remote cleanup.
8. Returning from the stream route closes the decoder/input/audio resources and returns to the React management UI.

A dedicated Android activity is acceptable as an embedded product route. It must not become a separate product window with an independent session model.

### 7.2 Streaming implementation options

Validate these options in the first native spike:

#### Option A: Reuse the existing Rust Moonlight session and add Android media adapters

- Keep No Land's current Rust GameStream session and packet/session state.
- Expose decoded or decodable video/audio packet callbacks to Kotlin/Java.
- Feed video access units into `MediaCodec` using a `Surface`.
- Feed Opus/PCM audio into Android's audio output path.
- Route Android input events into the existing No Land input manager.

This is preferred if the current Rust session already owns the required network and packet timing behavior.

#### Option B: Use a thin Android native Moonlight core adapter

- Reuse `moonlight-common-c` and the protocol behavior from Moonlight Android.
- Wrap only the connection/session callbacks required by No Land.
- Keep the No Land Rust commands and state model above the native adapter.

Use this only if Option A cannot provide stable Android decoder/input timing or required codec behavior. Avoid maintaining two independent GameStream implementations without a documented reason.

### 7.3 Video requirements

The Android video adapter must support, as device capabilities permit:

- H.264 baseline/high profiles;
- HEVC;
- AV1 only after an explicit device capability gate;
- codec-specific configuration data and access-unit boundaries;
- decoder flush/restart after packet loss or IDR recovery;
- surface replacement after activity recreation;
- rotation and aspect-ratio changes;
- HDR metadata where supported;
- decoder output timing and queue-depth metrics;
- clean fallback/error reporting for unsupported profiles;
- reconnect without leaking codec instances or surfaces.

Use `MediaCodec` and the Moonlight Android decoder logic as the primary reference. Do not decode frames in JavaScript or send full video frames through the Tauri WebView.

### 7.4 Audio requirements

Implement Android output using `AudioTrack` or the best supported low-latency Android API after measurement. Preserve:

- Opus stream handling;
- channel count and sample-rate negotiation;
- audio buffering and underrun metrics;
- route changes between speaker, wired audio, Bluetooth, and USB audio;
- pause/resume and focus handling;
- stream teardown on activity stop or remote disconnect.

Microphone forwarding is a separate capture path using `AudioRecord` or an equivalent native API. It must preserve No Land's existing host-agent/session protocol, mute state, bitrate configuration, permissions, interruption recovery, and cleanup behavior.

### 7.5 Input requirements

Use Moonlight Android as the reference for:

- absolute and relative touch modes;
- touch gestures and virtual controller controls;
- Android key event to remote keyboard translation;
- hardware mouse and scroll events;
- physical game controllers and Android TV input;
- controller rumble/haptics where available;
- focus loss and input capture release;
- orientation and multi-window behavior.

The native adapter should translate Android events to a small stable Rust input interface instead of duplicating the entire No Land input state machine in Kotlin.

### 7.6 Stream controls

The native stream route must expose:

- return/disconnect;
- microphone mute and status;
- clipboard send/get through explicit user actions;
- network warning and reconnect state;
- FPS, decode, audio, and latency diagnostics;
- touch/input mode selection;
- virtual-controller visibility;
- stream settings that are safe to change during a session.

React may render management screens before and after streaming. Native controls should own latency-sensitive interactions and controls that must remain available when the WebView is covered or paused.

## 8. Android networking and VPN

### 8.1 WireGuard

Replace the iOS Network Extension implementation with an Android `VpnService`-based adapter or a maintained Android WireGuard integration.

The adapter must preserve the current connection-manager contract:

- one logical WireGuard tunnel with host target `10.77.0.1`;
- Direct and Cloudflare TURN as alternative outer transports;
- Auto selection, manual selection, repair, and rollback;
- runtime identity/fingerprint read-back;
- endpoint and MTU mutation;
- handshake and traffic counters;
- allocation expiry and renewal;
- serialized mutations and durable transaction state;
- recovery after app process death, VPN service restart, network change, and cellular/Wi-Fi transitions.

The Android VPN implementation must not assume that the Rust/Tauri process remains alive while the app is backgrounded. Decide which maintenance tasks belong in the VPN service and which are reconciled when the app returns to the foreground.

### 8.2 Permissions and transport

Document and test:

- VPN consent flow;
- foreground-service notification requirements;
- local-network access and mDNS only when needed;
- Wi-Fi/cellular changes;
- restricted/background modes;
- Android battery optimization behavior;
- clear user-visible failure states when the OS stops the VPN service.

## 9. Android secure storage and filesystem

Replace Apple Keychain and iOS document APIs with:

- Android Keystore-backed encryption for Vast/API credentials, SSH material, OAuth refresh tokens, Cloudflare tokens, and WireGuard references;
- encrypted preference/state storage for opaque references and non-secret state;
- Storage Access Framework for user-selected upload/import/export files;
- content-URI streaming rather than permanent filesystem-path assumptions;
- scoped storage-compatible diagnostics export and share flows;
- migration from desktop-origin state and the existing mobile reference state where applicable.

Secrets must not be placed in the WebView local storage, APK resources, Gradle files, logs, or environment variables shipped to production.

## 10. Android UI and lifecycle adaptation

Port the current screens rather than replacing the product UI with Moonlight's UI.

Required UI work:

- single-column phone layouts;
- tablet and foldable layouts;
- large touch targets and Android back navigation;
- keyboard and IME avoidance;
- safe-area/inset handling;
- bottom sheets instead of desktop-sized modals;
- Android permission prompts with contextual explanations;
- foreground/background resume reconciliation;
- native share and file-picker flows;
- stream orientation policy, normally portrait management UI and landscape streaming;
- accessibility and TalkBack labels;
- dark theme and system bar handling.

Lifecycle events must be explicit:

- `onCreate` / process recreation;
- `onStart` / `onStop`;
- `onResume` / `onPause`;
- configuration and orientation changes;
- activity result callbacks;
- VPN service restart;
- decoder surface destruction/recreation;
- audio focus loss and restoration;
- app suspension during provisioning or upload.

## 11. Proposed repository structure

```text
no_land_android/
  src/                         # shared React/Tauri UI
  src-tauri/
    src/
      commands/
      models/
      services/
      platform/
        android.rs
      moonlight/
        domain/
        infrastructure/
        platform/
          android.rs
      android_bridge.rs
    android/                   # source-controlled Android native module/templates
      app/
        src/main/AndroidManifest.xml
        src/main/java/com/noland/connect/
          NolandMainActivity.kt
          NolandStreamActivity.kt
          NolandTauriPlugin.kt
          stream/
            NolandStreamView.kt
            MediaCodecVideoRenderer.kt
            AndroidAudioRenderer.kt
            AndroidInputDispatcher.kt
            StreamLifecycleController.kt
          vpn/
            NolandVpnService.kt
            WireGuardController.kt
          storage/
            AndroidSecureStore.kt
            AndroidDocumentStore.kt
        src/main/cpp/
          noland_android_jni.cpp
          moonlight_adapter.cpp
      build.gradle
      proguard-rules.pro
    native-templates/
    tauri.mobile.android.conf.json
  scripts/
    configure-android-project.mjs
    check-android.mjs
    check-android-streaming.mjs
    check-android-contracts.mjs
  docs/
    android-port-status.md
    android-streaming-reference.md
    android-vpn-reference.md
  ANDROID_PORT_PLAN.md
```

The actual Tauri-generated paths may differ. The invariant is that generated output is reproducible and the Android native layer is isolated from shared Rust/domain logic.

## 12. Work phases

### Phase 0: Repository and legal/build spike

- [ ] Create the new repository from the selected `no_land_mobile_port` revision.
- [ ] Preserve an upstream remote for syncing product changes.
- [ ] Add Android-specific Tauri configuration without changing desktop defaults.
- [ ] Generate a clean Tauri Android project.
- [ ] Confirm Android Studio, SDK, NDK, Gradle, Java, Rust Android targets, and device/emulator setup.
- [ ] Confirm GPL-3 compatibility and attribution requirements for any Moonlight Android source or dependency used.
- [ ] Decide whether Moonlight Android code will be used as a pinned source dependency, a vendored isolated adapter, or only as a behavioral reference.
- [ ] Produce a minimal Android APK showing the React management UI.

**Exit condition:** clean checkout builds and installs a debug APK on one physical Android device without any streaming code.

### Phase 1: Android platform abstraction

- [ ] Split current `moonlight/platform` assumptions from desktop window/input behavior.
- [ ] Add Android capability reporting.
- [ ] Add Android lifecycle events and foreground/background reconciliation.
- [ ] Add Android logging and diagnostic export.
- [ ] Add Android secure-store and document-provider interfaces.
- [ ] Add explicit typed unsupported errors for not-yet-ported native operations.
- [ ] Keep desktop and existing iOS builds passing.

**Exit condition:** all shared screens compile for Android and every Android-specific operation has an explicit adapter boundary.

### Phase 2: Android native stream proof of concept

- [ ] Create a native Android stream route with a `Surface`.
- [ ] Establish the Rust-to-Kotlin/JNI event and command boundary.
- [ ] Feed a test H.264 stream into `MediaCodec`.
- [ ] Render decoded frames without passing video through React.
- [ ] Implement basic frame timing and decoder reset.
- [ ] Implement basic Opus/audio output.
- [ ] Add instrumentation for decoder queue, render time, dropped frames, and audio underruns.

**Exit condition:** a controlled test stream renders and plays audio on at least one physical arm64 device for 30 minutes without leaks or increasing latency.

### Phase 3: Real Moonlight session integration

- [ ] Select Option A or Option B from the streaming design.
- [ ] Reconcile No Land's current Moonlight session with the Android native decoder contract.
- [ ] Support host pairing and application launch through the existing No Land flow.
- [ ] Support H.264 and HEVC negotiation.
- [ ] Support packet loss, IDR recovery, reconnect, stop, and remote disconnect.
- [ ] Preserve stream metrics and network-warning events.
- [ ] Support orientation, surface recreation, and activity return.

**Exit condition:** a real Sunshine host can pair, launch, stream, return, and reconnect on a physical device.

### Phase 4: Input, audio, and controls

- [ ] Implement touch absolute/relative modes.
- [ ] Implement Android keyboard translation.
- [ ] Implement mouse, scroll, and game-controller input.
- [ ] Implement virtual-controller controls.
- [ ] Implement controller rumble where supported.
- [ ] Implement audio focus, route changes, and Bluetooth behavior.
- [ ] Implement microphone permission, capture, mute, encoding, and forwarding.
- [ ] Implement native stream controls and clipboard actions.

**Exit condition:** the device can play a representative game using touch, hardware controller, keyboard/mouse where available, audio, microphone, and clipboard without leaving the stream route.

### Phase 5: VPN and remote-management parity

- [ ] Implement Android WireGuard/VPN service.
- [ ] Preserve Direct/TURN/Auto selection and rollback behavior.
- [ ] Implement foreground-service and permission flows.
- [ ] Implement SSH/provisioning lifecycle recovery.
- [ ] Implement Android secure storage migration.
- [ ] Implement Storage Access Framework uploads and shared-storage operations.
- [ ] Implement native share/diagnostics export.
- [ ] Test process death, VPN restart, network changes, and app resume.

**Exit condition:** a new user can onboard, rent/provision an instance, establish the managed connection, stream, stop, and resume after Android lifecycle interruptions.

### Phase 6: Product hardening and release

- [ ] Test phone, tablet, foldable, and Android TV/large-screen targets if included in scope.
- [ ] Test multiple Android API levels and GPU vendors.
- [ ] Run release shrinking/obfuscation and native symbol upload checks.
- [ ] Add Play App Signing and CI without committing signing secrets.
- [ ] Add crash diagnostics and native tombstone collection with redaction.
- [ ] Validate privacy declarations, microphone/VPN disclosures, and foreground-service declarations.
- [ ] Produce a signed internal-test AAB.
- [ ] Run staged internal testing before public release.

**Exit condition:** release candidate passes the complete acceptance matrix and no required workflow is hidden, mocked, or permanently disabled.

## 13. Validation matrix

### Build and contract validation

- `npm run build`
- `npm run i18n:check`
- Android TypeScript/Rust command-contract check
- Rust unit and integration tests
- Android debug APK build from a clean checkout
- Android release AAB build with native symbols
- arm64 Rust and native Android builds
- generated-project reproducibility check

### Streaming validation

- H.264 stream
- HEVC stream
- supported HDR path
- decoder initialization failure
- malformed codec configuration
- packet loss and IDR recovery
- reconnect after Wi-Fi loss
- surface destruction/recreation
- activity rotation
- background/foreground transition
- audio underrun and route changes
- Bluetooth headset transition
- microphone permission denial and interruption
- touch, keyboard, mouse, controller, and virtual-controller input
- clipboard send/get
- stream stop from Android back gesture and native control

### Network/VPN validation

- Direct transport
- Cloudflare TURN transport
- Auto transport selection
- failed transition and rollback
- expired allocation/credentials
- VPN permission denial
- VPN service restart
- process death
- Wi-Fi to cellular transition
- cellular to Wi-Fi transition
- host reachability and control RPC validation
- provisioning while app is backgrounded or resumed

### Device matrix

At minimum, test:

- one current arm64 Pixel-class phone;
- one Samsung-class phone;
- one lower/mid-range Android device;
- one tablet or foldable layout;
- one device with a different GPU/vendor decoder implementation;
- Android emulator for UI and contract checks, not as the only streaming validation target.

## 14. Acceptance criteria

The Android port is not complete because the APK builds or because the existing Tauri commands remain registered. Completion requires:

1. onboarding and secure credential storage work;
2. Vast.ai discovery and instance management work;
3. provisioning works through Android-compatible SSH/network paths;
4. Direct and TURN connection behavior remains correct;
5. WireGuard/VPN state survives normal Android lifecycle events;
6. Moonlight pairing and application launch work;
7. embedded H.264/HEVC video and audio work on supported devices;
8. touch, controller, keyboard, mouse, microphone, and clipboard behavior is tested;
9. uploads, shared storage, terminal, settings, diagnostics, and recovery flows have Android equivalents;
10. failures are visible and recoverable;
11. release artifacts are reproducible and signed through CI;
12. all required workflows are reachable without desktop-only assumptions.

## 15. Risks and decisions

### Native media scope

The largest risk is building a stable Android media adapter. Moonlight Android's mature `MediaCodec` and input code should reduce discovery work, but it does not remove the need to reconcile its callback/session model with No Land's existing Rust stream implementation.

### Duplicate protocol implementations

No Land already contains Moonlight/GameStream code and the native `moonlight-common-c` dependency. Copying Moonlight Android's complete `NvConnection` and protocol stack would create two sources of truth. Choose one session owner and document the boundary before implementation.

### VPN implementation

Android `VpnService` lifecycle and background restrictions differ substantially from desktop and iOS. The implementation must be tested on real devices and must not assume that JavaScript timers or the Tauri process run while suspended.

### Licensing and attribution

Moonlight Android and related components are GPL-licensed. Before copying or vendoring source, record the exact upstream revision, preserve notices, include required source/license distribution, and have the final dependency/license arrangement reviewed before distribution.

### Product identity

Before release, choose a new Android application ID and signing identity. Do not reuse the Moonlight application ID, package name, signing key, branding, or update channel.

## 16. Initial implementation order

The recommended order is:

1. New repository copy and reproducible Tauri Android shell.
2. Platform capability/lifecycle/storage abstractions.
3. Native stream surface and Rust/Kotlin bridge spike.
4. `MediaCodec` H.264 proof of concept.
5. Real Moonlight session integration.
6. Audio and input.
7. Android VPN service.
8. Provisioning, uploads, terminal, and lifecycle recovery.
9. Device matrix and release hardening.

Do not begin by copying the entire Moonlight Android app. Start by proving the narrowest useful boundary: No Land starts one real Moonlight session, Android owns a native `Surface`, `MediaCodec` decodes the stream, and the user can return cleanly to the No Land dashboard.
