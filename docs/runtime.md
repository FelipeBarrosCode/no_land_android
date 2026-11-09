# Modern Linux Runtime — mobile preview

Android controls the same optional **remote Linux** runtime as the No Land desktop
client. Ubuntu 24.04, AppImages and native ELF files execute on the rented x86-64
Linux PC, **never on the phone**. This is not an Android Linux container, an APK
installer, or an alternative game-streaming pipeline.

## Usage

Open an instance's launch library → **Modern Linux Runtime**:

1. Enable the remote companion (optional, independent of normal desktop access).
2. Review measured download/free-disk sizes **on the remote PC**, then approve
   installation. Consent is tied to the reviewed artifact SHA256.
3. Enter the absolute path of a trusted Linux executable/AppImage on that PC.
   There is intentionally no Android file picker: `content://`/`file://` paths
   are not remote executables.
4. Launch the registered app through its runtime library card. Per-app shortcut
   buttons create shortcuts on the **remote desktop**, not the Android launcher.
5. Use diagnostics, cancel/retry or rollback as needed. Native and Steam/Proton
   routes stay unchanged. Runtime errors never silently retry on the parent OS
   or fall back to PRoot.

The touch layout stacks controls on narrow screens and uses 44px minimum touch
targets. Status polling starts only while the panel is expanded, pauses when the
app is hidden, ignores stale responses after closing, and refreshes on return.
Authorized installs and launched games are supervised remotely and are not
cancelled merely because Android backgrounds the app or the SSH request ends.
The reviewed download plan is cleared on visibility changes. A companion upload
interrupted before installation may need **Enable** again.

## Shared signed release setup

This client reuses the engine, artifact builder and native GTK companion from
[`FelipeBarrosCode/no_land`](https://github.com/FelipeBarrosCode/no_land), built by
its `runtime-artifacts.yml` workflow on Ubuntu 22.04. Do not build an Android
version of that Linux companion or execute its downloaded bytes on the phone.
The mobile application only downloads a pinned, size-limited companion to memory,
verifies SHA256, and transfers it over existing in-process SSH to the Linux PC.

Configure these four **public repository variables** for the Android workflow,
or export them before building locally:

```text
NOLAND_RUNTIME_AGENT_URL=https://…/<immutable-tag>/noland-runtime-linux-x86_64
NOLAND_RUNTIME_AGENT_SHA256=<64-hex SHA256 from the verified release>
NOLAND_RUNTIME_MANIFEST_URL=https://…/<immutable-tag>/manifest.json
NOLAND_RUNTIME_PUBLIC_KEY=<64-hex Ed25519 public key verified out-of-band>
```

Never include a private signing key in the mobile repository/application. Do not
use mutable `latest` URLs. Without all four valid pins, enabling stays disabled;
an already-installed compatible companion can still be managed. The large base
image is downloaded/verified/extracted by the remote engine, not through Android.
No new device storage permissions, shell binaries, public services or ports are
required.

The remote parent must already have `zenity`, `bwrap`, `xauth`, `curl`, `sudo`,
`visudo`, `timeout`, `ldconfig`, `ldd`, `sha256sum`, matching injected NVIDIA
graphics/display components, the existing No Land X11 session and audio endpoint.
Enabling requires root/passwordless administrative SSH. It installs only the
pinned companion, constrained credential helper/sudoers entry, release metadata
and desktop shortcuts; it does not run apt, change libc/drivers, or restart
Xorg/Sunshine. No Linux rootfs/image files are packaged as Android resources.

## Security and remaining gates

Sharing trusted X11 is **not a security sandbox** for arbitrary downloads. The
remote engine refreshes private Xauthority, keeps per-app saves outside images,
checks namespaces/glibc/hardware rendering/audio and refuses unsupported native
backends. Credentials/driver bindings/images/caches are excluded from the
updated shared-storage file-attribution rules; rebuild the remote state-agent
with this source change before relying on those exclusions. Automatic runtime-app
shared-storage restoration is still unfinished; keep manual backups of remote
registry and `app-data`, never private sessions or machine-specific bindings.

This has the same preview limits as desktop: NVIDIA x86-64/Bubblewrap only; no
PRoot, 32-bit, Steam nesting, Windows compatibility or anti-cheat workaround.
Lichii's original assertion is **unvalidated**, not claimed fixed.

Before shipping, configure/publish a signed runtime release and test on actual
Android hardware plus a Vast.ai parent: background/foreground, lost network and
reconnect, interrupted enable/install, streaming GPU/audio/controllers, AppImages,
rollback with running apps, narrow layouts and keyboard/safe-area behavior.
Rust cross-checks with native media skipped do not produce or validate an APK.

## Checks

```sh
npm run test:runtime
npm run build
npm run i18n:check
npm run check:android
```

The runtime tests cover consent metadata, literal remote paths, polling lifecycle,
late results/errors, command registration/SSH boundaries and all nine locales.
