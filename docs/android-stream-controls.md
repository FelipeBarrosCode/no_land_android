# Android in-stream controls

The Android controls follow the active implementation in `no_land_mobile_port`'s
`noland_stream_controls_ios.m` and `noland_video_renderer_ios.m`. They are native
Android views above the decoder Surface, not the legacy WebView stream screen.

## Drawer

Swipe right from the left safe-area strip to open **Stream controls**. There is no
permanent hamburger over the video. Drag left, tap outside the drawer, press Back,
or select **Resume stream** to close it. The drawer scrolls on short displays.

The drawer contains:

- **Trackpad**, **Direct touch**, and **Click-to-use**, with mode-specific hints.
- **On-screen controller**.
- **Keyboard**.
- **Return to Noland** (keeps the connection alive).
- **Resume stream** (dismisses the drawer).

Touch mode and controller visibility persist app-wide in SharedPreferences, using
`noland.stream.touchMode` and `noland.stream.gamepad`. Defaults are Trackpad and
controller off. Opening the drawer releases held input without unplugging player 1.

## Fingers

| Mode | One finger | Two fingers |
| --- | --- | --- |
| Trackpad | Slide moves the cursor relatively; tap left-clicks. Hold still for 650 ms, then move to drag. | Slide scrolls vertically; stationary tap right-clicks. |
| Direct touch | Contact presses left at the absolute position; movement drags. Hold still for 650 ms to switch to right-button press. | Slide scrolls vertically. A two-finger tap does **not** right-click in this mode, matching iOS. |
| Click-to-use | Contact/sliding positions the cursor without a held button. A stationary tap clicks there; hold still for 650 ms, then drag. | Slide scrolls vertically; stationary tap right-clicks. |

A **three-finger tap outside the left-edge strip opens the keyboard**, not the
menu. It also works in empty areas while the on-screen controller is enabled;
touches starting on controller widgets are excluded. Mouse gestures remain disabled
throughout controller mode, including empty areas.

Movement threshold is 5 dp. Synthetic tap releases occur after 50 ms. Pointer IDs
are tracked across reordered events, and one finger remaining after a multi-finger
gesture cannot accidentally resume a drag. Cancellation, focus loss, keyboard,
mode changes, and session retirement release buttons and invalidate timers.
Absolute positions respect the aspect-fit video and letterboxing.

## Keyboard

Only the drawer action or three-finger gesture authorizes the offscreen IME target;
ordinary video/controller taps never open it. A 44 dp toolbar provides **Done**.
Back dismisses the keyboard before it can disconnect the stream. The decoder
Surface is not resized for the IME.

Committed ASCII uses US virtual-key pairs; Unicode uses the native UTF-8 protocol.
IME composition stays local until committed. Text, Backspace, Enter, and hardware
key edges share one generation-tagged FIFO so editing operations cannot overtake
one another. Suggestions, autocorrection, fullscreen extraction, and personalized
learning are disabled where the Android API supports it.

Physical keyboard mapping includes letters, numbers, punctuation, F1–F12, arrows,
navigation/editing keys, numeric keypad, lock keys, and left/right modifiers.
Android's public key-code set does not expose iOS HID F13–F24 equivalents.

## Controllers and physical mouse

The virtual controller provides two sticks, diagonal D-pad, ABXY, LB/RB, digital
LT/RT, L3/R3, Select, and Start. Each widget owns its touch independently. Sticks
use the iOS square travel/response behavior without a client deadzone.

Physical gamepads use stable player slots, hotplug handling, analog triggers, and
standard extended buttons. The first physical controller and virtual controller
share player 1: buttons are ORed, triggers use the maximum, and each axis uses the
strongest magnitude (physical wins ties). Player 1 remains present across menu,
focus, and overlay changes. Failed arrival reports retry, including after runtime
reconnect. Suspended input still observes received physical release events.

Physical mice support absolute movement, five buttons, and vertical/horizontal
high-resolution wheel input. Captured-relative events are supported by the input
adapter; the current UI does not force pointer capture automatically. Drawer
navigation remains local while remote input is suspended.

This does **not** claim iOS-specific controller LED, battery, motion-sensor, or
rumble feedback parity. Those outputs require separate Android device adapters.
Physical Bluetooth/USB hardware acceptance is also separate from synthetic tests.

## Dashboard and statistics

**Return to Noland** relinquishes native input and hides the presentation while
preserving the runtime and remote instance. The dashboard shows **Return to stream**
and **Disconnect**. Return recreates/reattaches a Surface under the same session;
it does not send a fresh launch/resume request. Disconnect stops the local stream,
not the rented VM or remote application.

The dashboard's existing **Performance overlay** switch controls native top-right
statistics during play. Updates are bounded to 250 ms, target only the current
route/Activity, and cannot recreate an old Activity. Unavailable measurements use
`—`. Android clipboard buttons are not exposed because the Android clipboard
adapter is not implemented; the existing iOS clipboard actions remain unchanged.

## Validation

Device-free regression commands:

```sh
npm run check:android
python3 src-tauri/android/tests/run_stream_touch_tests.py
python3 src-tauri/android/tests/run_stream_keyboard_tests.py
python3 src-tauri/android/tests/run_stream_hardware_pure_tests.py
python3 src-tauri/native/noland-moonlight/tests/run_mobile_input_release_tests.py
python3 src-tauri/native/noland-moonlight/tests/run_android_video_renderer_tests.py --sanitize
```

The Kotlin runners use the installed/cached compiler without downloading packages.
Rust coordinator tests live beside the implementation in `platform/android.rs`.

On the connected Pixel 7a, verify the drawer, all mode selections, controller
visibility, three-finger keyboard (including empty controller areas), Done, and a
round trip to the dashboard lasting longer than five seconds. Confirm that frames
continue after Return to stream and that native start count has not increased.
`InjectStreamTouch.java` is an optional **shell-only** two/three-finger tap helper;
compile it against the Android SDK, convert its class with `d8`, and run the dex
through `adb shell app_process`. It is not packaged or executable from app UI.

The live smoke run covered native keyboard opening/Done, persisted mode/controller
preferences, overlay rendering, and dashboard reentry without a new stream start.
It is not sustained-performance or physical-controller/haptics acceptance.
