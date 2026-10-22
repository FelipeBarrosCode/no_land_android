# Android emulator compatibility audit

Audit date: 2026-10-11

> Historical audit: native H.264 streaming, playback audio, touch/basic keyboard input, JNI surface ownership, and WireGuard VPN were subsequently implemented and smoke-tested on a physical ARM64 Pixel 7a on 2026-10-21. See [current Android port status](android-port-status.md). The matrix below records the earlier emulator build, not current source support.

Target used for this audit:

- AVD: `No_Land_API_36`
- Android: 16 / API 36
- ABI: `x86_64`
- App package: `com.noland.connect`
- App version: `0.1.17`
- Minimum SDK declared by the installed APK: API 24
- Target SDK declared by the installed APK: API 36

## Conclusion

The API 36 x86_64 emulator can run and test the No Land **management shell**, shared frontend, Rust backend, network API calls, app-private state, and basic Android lifecycle. It cannot currently run the complete product because several Android implementations are absent or intentionally stubbed.

This is primarily an implementation gap, not an emulator limitation. Real streaming, managed VPN, audio, microphone forwarding, native stream input, and clipboard cannot be accepted until their Android adapters are connected.

## Requirements matrix

| Requirement | Emulator capability | Current app status | Result |
| --- | --- | --- | --- |
| APK installation and launch | Supported | x86_64 APK installs; `MainActivity` reaches `RESUMED` | Pass |
| Tauri WebView and React UI | Supported | Onboarding UI renders and leaves the loading screen | Pass |
| x86_64 Rust/native library | Supported | `libnoland_connect.so` loads successfully | Pass |
| ARM64 Rust/native library | Not validated by this x86_64 APK | CI builds ARM64, but no ARM64 runtime test has run | Physical device or ARM64 emulator required |
| Internet, DNS, TLS, HTTP | Supported | Rust HTTP requests and Vast.ai API access work | Pass for connectivity |
| Vast.ai onboarding and management | Supported | API calls can run; paid provisioning was not performed in this audit | Partial; external account and billable test required |
| In-process SSH/SFTP | Supported | Android selects `noland-ssh`; a real host session was not exercised | Partial; reachable host required |
| App state persistence | Supported | `state.json` persists across cold starts | Pass |
| Secret persistence | Supported | AES-256-GCM app-private vault persists and scrubs plaintext from `state.json` | Pass for emulator storage |
| Hardware-backed Android Keystore | Emulator can expose Android Keystore APIs | The current vault key is app-private, not Keystore-wrapped | Not implemented |
| Notifications | Supported | Plugin is packaged; `POST_NOTIFICATIONS` is currently denied | Permission-flow test required |
| Touch and software keyboard | Supported | Management UI can receive emulator input | Pass for basic UI input |
| Stream activity and surface lifecycle | Supported | Activity, `SurfaceView`, and `MediaCodec` shell exist | Partial; JNI/session path is disconnected |
| H.264 decoding | Goldfish hardware-accelerated decoder is advertised | Decoder wrapper exists but no stream packets reach it | Blocked by stream integration |
| HEVC decoding | Goldfish hardware-accelerated decoder is advertised | Decoder wrapper exists but no stream packets reach it | Blocked by stream integration |
| AV1 decoding | Software decoder is advertised | No implemented stream integration or capability policy | Not ready |
| Native Moonlight/GameStream session | Emulator networking and codecs are sufficient for functional development | Android links `noland_android_runtime_stub.c`; stream start returns invalid state | Not implemented |
| Kotlin/Rust surface JNI | Supported | Kotlin declares JNI methods, but matching native exports are absent | Not implemented |
| Video performance, HDR, 10-bit, vendor codecs | Emulator cannot represent production hardware accurately | No end-to-end stream exists | Physical devices required |
| Audio output | Emulator exposes a speaker route and Opus software decoding | No Android `AudioTrack`/native renderer is connected | Not implemented |
| Microphone capture | Emulator advertises a microphone | Permission is denied and the app has no Android request/capture adapter | Not implemented |
| Hardware mouse and relative capture | Limited emulation possible | Native stream input returns unsupported | Not implemented |
| Game controllers and Android TV remote | Can be simulated or attached, but not equivalent to full hardware testing | Controller JNI/input path is stubbed | Not implemented; physical-device follow-up required |
| Haptics/rumble | Poor emulator fidelity | No connected controller output path | Physical device required |
| Android managed VPN/WireGuard | Emulator supports `VpnService` | No `VpnService` or Android WireGuard adapter is declared | Not implemented |
| Direct/TURN transport switching | Network can be emulated | Depends on the missing managed VPN and live infrastructure | Not runnable |
| Clipboard | Android supports `ClipboardManager` | Rust returns an explicit native-bridge-required error | Not implemented |
| File picker/import/export | Emulator supports Storage Access Framework | No maintained Android SAF/content-URI adapter | Not implemented |
| Diagnostic sharing | Emulator supports `FileProvider` and share intents | Provider is declared, but the native share workflow is incomplete | Partial |
| Display detection | Emulator exposes display metrics | Rust has no Android display-detection branch | Not implemented |
| Keep screen awake | Supported | Stream activity uses keep-screen-on behavior | Partial; only the stream shell uses it |
| Foreground streaming service | Supported | No foreground service or process-death reconciliation exists | Not implemented |
| Background restrictions and OEM task killing | Only partially reproducible | No Android service/reconciliation implementation | Physical OEM-device testing required |
| Bluetooth/USB/wired audio routing | Poor emulator fidelity | Android audio path is not implemented | Physical device required |
| Cellular/Wi-Fi transitions | Basic network controls can be simulated | Managed transport lifecycle is not implemented | Physical device and network tests required |
| API 24 minimum compatibility | Requires an API 24 AVD | Only API 36 runtime was tested | Additional emulator required |
| Tablet/foldable/Android TV layouts | Requires dedicated AVD profiles | Current test uses a 320x640 phone profile | Additional AVDs required |
| Release signing and Play bundle | Emulator can install signed release APKs | Current artifacts are debug APKs | Not implemented |

## Evidence

### Runtime checks that passed

- The installed package resolves `com.noland.connect/.MainActivity`.
- The process remains alive, visible, fully drawn, and resumed.
- The APK selects `x86_64` as its primary ABI.
- Rust logs report state loading and the Android shell runtime initialization.
- The app leaves the loading screen and renders onboarding.
- Secure state survives a second cold start.
- The supplied Vast.ai API key was restored from encrypted storage, matched without being printed, and could be saved again.
- The plaintext `credentials.vastApiKey` field in `state.json` remains empty.
- Android CI builds and uploads separate ARM64 and x86_64 debug APKs.

### Runtime permissions

The installed API 36 package currently reports:

| Permission | State |
| --- | --- |
| `INTERNET` | Granted |
| `WAKE_LOCK` | Granted |
| `RECEIVE_BOOT_COMPLETED` | Granted |
| `RECORD_AUDIO` | Denied |
| `POST_NOTIFICATIONS` | Denied |

Declaring a permission in `AndroidManifest.xml` is not enough. Microphone and notification flows must request runtime permission and handle denial explicitly.

### Decisive source blockers

- `src-tauri/native/android/noland_android_runtime_stub.c` identifies itself as `android-shell` and rejects stream and input operations.
- `src-tauri/src/moonlight/platform/window.rs` reports that Android stream presentation and surface bridging are not connected.
- `src-tauri/android/app/src/main/java/com/noland/connect/stream/NativeStreamBridge.kt` declares JNI calls without matching native implementations.
- `src-tauri/src/services/clipboard.rs` returns a native-bridge-required error on Android.
- `src-tauri/src/mic_client/permissions.rs` does not request Android microphone permission.
- `src-tauri/android/app/src/main/AndroidManifest.xml` contains no Android VPN service.

## What the emulator can cover next

The emulator should remain the fast development target for:

1. APK install/launch smoke tests in CI.
2. Onboarding and responsive UI tests.
3. Runtime notification and microphone permission flows.
4. JNI symbol and stream-activity launch tests.
5. `SurfaceView` recreation, rotation, pause/resume, and process recreation.
6. `MediaCodec` H.264/HEVC configuration, packet queueing, flush, and decoder recovery.
7. Basic `AudioTrack` playback and `AudioRecord` capture plumbing.
8. Android clipboard, Storage Access Framework, share intents, and deep-link callbacks.
9. `VpnService` consent, startup, teardown, and process-restart behavior.
10. API-level testing at API 24, an intermediate supported API, and API 36.

## Tests that must not rely only on the emulator

Before release, use physical ARM64 phones/tablets and, if supported, Android TV hardware to validate:

- Qualcomm, MediaTek, and Exynos codec behavior;
- sustained 1080p/1440p/4K latency and frame pacing;
- HDR, 10-bit HEVC, refresh-rate switching, and wide color;
- thermals, battery consumption, memory pressure, and background restrictions;
- Bluetooth, wired, and USB audio route changes;
- real microphone latency and interruption recovery;
- hardware mouse capture, keyboards, controllers, rumble, and haptics;
- Wi-Fi/cellular transitions and VPN recovery;
- hardware-backed Keystore and StrongBox behavior;
- OEM task killers and lock-screen behavior.

## Release gate

The current emulator build is suitable for management UI and platform-integration development. It is **not release-ready as a streaming client**. The minimum critical path is:

1. replace the Android Moonlight shell runtime;
2. connect Rust/Kotlin JNI and the native stream activity;
3. implement Android `VpnService`/WireGuard transport;
4. implement video, audio, input, and microphone adapters;
5. implement runtime permissions and lifecycle recovery;
6. add emulator installation/instrumentation CI;
7. complete physical ARM64 device acceptance.
