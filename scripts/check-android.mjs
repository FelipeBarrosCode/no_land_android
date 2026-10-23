import { existsSync, lstatSync, readFileSync, readdirSync } from "node:fs";
import { basename, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import {
  DESUGAR_DEPENDENCY,
  injectWireGuardDependency,
  WIREGUARD_DEPENDENCY,
} from "./configure-android-project.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const failures = [];
const checks = [];

function check(condition, message) {
  if (condition) {
    checks.push(message);
  } else {
    failures.push(message);
  }
}

function readJson(relativePath) {
  try {
    return JSON.parse(readFileSync(join(root, relativePath), "utf8"));
  } catch (error) {
    failures.push(`${relativePath} must contain valid JSON: ${error.message}`);
    return {};
  }
}

function hasOwn(value, key) {
  return Object.prototype.hasOwnProperty.call(value ?? {}, key);
}

const packageJson = readJson("package.json");
const androidConfig = readJson("src-tauri/tauri.android.conf.json");
const androidCapability = readJson("src-tauri/capabilities/android.json");
const configureScriptPath = join(root, "scripts", "configure-android-project.mjs");
const configureScript = existsSync(configureScriptPath)
  ? readFileSync(configureScriptPath, "utf8")
  : "";
const gitignore = readFileSync(join(root, ".gitignore"), "utf8");
const tauriBuildScript = readFileSync(join(root, "src-tauri", "build.rs"), "utf8");
const androidWorkflow = readFileSync(join(root, ".github", "workflows", "android.yml"), "utf8");
check(
  androidWorkflow.includes('ndk="${ANDROID_HOME}/ndk/${ANDROID_NDK_VERSION}"') &&
    androidWorkflow.includes('cmake_bin="${ANDROID_HOME}/cmake/${ANDROID_CMAKE_VERSION}/bin"') &&
    androidWorkflow.includes('echo "NDK_HOME=${ndk}"') &&
    androidWorkflow.includes('echo "ANDROID_NDK_HOME=${ndk}"') &&
    androidWorkflow.includes('echo "CMAKE=${cmake_bin}/cmake"') &&
    androidWorkflow.includes('>> "$GITHUB_ENV"') &&
    androidWorkflow.includes('echo "$cmake_bin" >> "$GITHUB_PATH"'),
  "Android CI selects the installed pinned NDK and CMake for Tauri and native builds",
);
check(
  tauriBuildScript.includes('native_env("ANDROID_PLATFORM")') &&
    tauriBuildScript.includes('native_env("NDK_API_LEVEL")') &&
    tauriBuildScript.includes('"android-24".into()') &&
    !tauriBuildScript.includes('native_env("ANDROID_API_LEVEL")'),
  "Android native minimum API is independent of the installed compile SDK level",
);
const stateAgentRuntime = readFileSync(
  join(root, "src-tauri", "src", "services", "shared_storage", "agent_runtime.rs"),
  "utf8",
);
const networkAgentRuntime = readFileSync(
  join(root, "src-tauri", "src", "services", "network_agent.rs"),
  "utf8",
);
const micReceiverRuntime = readFileSync(
  join(root, "src-tauri", "src", "services", "mic_receiver.rs"),
  "utf8",
);
const ignoreLines = new Set(
  gitignore
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#")),
);

const expectedScripts = {
  "configure:android": "node scripts/configure-android-project.mjs",
  "tauri:android:init":
    "tauri android init --ci --config src-tauri/tauri.android.conf.json && npm run configure:android",
  "tauri:android:dev":
    "npm run configure:android && tauri android dev --config src-tauri/tauri.android.conf.json",
  "tauri:android:build":
    "npm run configure:android && tauri android build --config src-tauri/tauri.android.conf.json",
  "check:android": "node scripts/check-android.mjs",
};

for (const [name, command] of Object.entries(expectedScripts)) {
  check(packageJson.scripts?.[name] === command, `package.json defines the expected ${name} command`);
}

check(androidConfig.identifier === "com.noland.connect", "Android identifier is com.noland.connect");
check(
  androidConfig.$schema === "https://schema.tauri.app/config/2",
  "Android config uses the Tauri 2 schema",
);
check(
  androidConfig.app?.windows?.some((window) => window.label === "main"),
  "Android config declares the main window",
);
check(
  Array.isArray(androidConfig.app?.security?.capabilities) &&
    androidConfig.app.security.capabilities.length === 1 &&
    androidConfig.app.security.capabilities[0] === "android",
  "Android config enables only the android capability",
);
check(!hasOwn(androidConfig.bundle, "iOS"), "Android config contains no iOS bundle settings");
check(!hasOwn(androidConfig.bundle, "macOS"), "Android config contains no macOS bundle settings");
check(
  androidConfig.bundle?.createUpdaterArtifacts === false,
  "Android updater artifacts are disabled",
);
check(
  Array.isArray(androidConfig.bundle?.externalBin) && androidConfig.bundle.externalBin.length === 0,
  "Android config excludes sidecars",
);
check(androidConfig.plugins?.updater === null, "Android config disables the updater plugin");

check(androidCapability.identifier === "android", "Android capability identifier is android");
check(
  Array.isArray(androidCapability.platforms) &&
    androidCapability.platforms.length === 1 &&
    androidCapability.platforms[0] === "android",
  "Android capability is restricted to Android",
);
check(
  Array.isArray(androidCapability.windows) &&
    androidCapability.windows.length === 1 &&
    androidCapability.windows[0] === "main",
  "Android capability is restricted to the main window",
);

const resources = androidConfig.bundle?.resources;
check(
  Array.isArray(resources) && resources.length === 0,
  "Android config avoids duplicate filesystem resources that are inaccessible on-device",
);

check(
  tauriBuildScript.includes("embed_android_remote_agent_sources") &&
    tauriBuildScript.includes('"state-agent-source.tar.gz"') &&
    tauriBuildScript.includes('"network-agent-source.tar.gz"') &&
    tauriBuildScript.includes('"mic-receiver-source.tar.gz"'),
  "Android build embeds compact archives for every remote agent source tree",
);
check(
  stateAgentRuntime.includes("EMBEDDED_STATE_AGENT_SOURCE") &&
    stateAgentRuntime.includes("materialize_embedded_state_agent_source"),
  "Android runtime materializes the embedded state-agent source archive",
);
check(
  networkAgentRuntime.includes("EMBEDDED_NETWORK_AGENT_SOURCE") &&
    networkAgentRuntime.includes("materialize_embedded_network_agent_source"),
  "Android runtime materializes the embedded network-agent and contracts source archive",
);
check(
  micReceiverRuntime.includes("EMBEDDED_MIC_RECEIVER_SOURCE") &&
    micReceiverRuntime.includes("materialize_embedded_receiver_source"),
  "Android runtime materializes the embedded microphone receiver source archive",
);

if (Array.isArray(resources)) {
  const srcTauri = join(root, "src-tauri");
  for (const resource of resources) {
    const wildcardIndex = resource.search(/[?*[]/u);
    const stablePart = wildcardIndex === -1 ? resource : resource.slice(0, wildcardIndex);
    const resourcePath = resolve(srcTauri, stablePart);
    const projectPrefix = `${resolve(root)}${sep}`;
    check(
      (resourcePath === resolve(root) || resourcePath.startsWith(projectPrefix)) && existsSync(resourcePath),
      `Bundled resource source exists: ${resource}`,
    );
  }
}

check(Boolean(configureScript), "Android configure script exists");
check(
  configureScript.includes('join(root, "src-tauri", "gen", "android")') &&
    configureScript.includes("if (!existsSync(generated))") &&
    configureScript.includes("tauri:android:init"),
  "Configure script fails clearly when src-tauri/gen/android is missing",
);
check(
  configureScript.includes('join(root, "src-tauri", "android")') &&
    configureScript.includes("if (!existsSync(overlay))") &&
    configureScript.includes("cpSync(overlay, generated") &&
    configureScript.includes("recursive: true") &&
    configureScript.includes("force: true"),
  "Configure script idempotently applies the optional maintained overlay",
);
check(
  !/child_process|ANDROID_HOME|ANDROID_SDK_ROOT/u.test(configureScript),
  "Configure script does not introduce an implicit SDK operation",
);
check(
  configureScript.includes(WIREGUARD_DEPENDENCY) &&
    configureScript.includes(DESUGAR_DEPENDENCY) &&
    configureScript.includes("injectWireGuardDependency") &&
    configureScript.includes('join(generated, "app", "build.gradle.kts")') &&
    configureScript.includes('join(generated, "app", "build.gradle")'),
  "Configure script targets either generated app Gradle dialect with pinned WireGuard and desugaring dependencies",
);

const kotlinGradleFixture = `plugins {\n  id("com.android.application")\n}\n\nandroid {\n  compileOptions {\n    sourceCompatibility = JavaVersion.VERSION_1_8\n    targetCompatibility = JavaVersion.VERSION_1_8\n    isCoreLibraryDesugaringEnabled = false\n  }\n  kotlinOptions {\n    jvmTarget = "1.8"\n  }\n}\n\ndependencies {\n  implementation("androidx.core:core-ktx:1.15.0")\n}\n`;
const configuredKotlinGradle = injectWireGuardDependency(kotlinGradleFixture, "kotlin");
check(
  configuredKotlinGradle.includes('implementation("androidx.core:core-ktx:1.15.0")') &&
    configuredKotlinGradle.includes(`implementation("${WIREGUARD_DEPENDENCY}")`) &&
    configuredKotlinGradle.includes(`coreLibraryDesugaring("${DESUGAR_DEPENDENCY}")`) &&
    configuredKotlinGradle.split(WIREGUARD_DEPENDENCY).length === 2 &&
    configuredKotlinGradle.split(DESUGAR_DEPENDENCY).length === 2,
  "Gradle injection preserves Kotlin DSL content and inserts one pinned WireGuard and desugaring dependency",
);
check(
  configuredKotlinGradle.includes("sourceCompatibility = JavaVersion.VERSION_17") &&
    configuredKotlinGradle.includes("targetCompatibility = JavaVersion.VERSION_17") &&
    configuredKotlinGradle.includes("isCoreLibraryDesugaringEnabled = true") &&
    configuredKotlinGradle.includes('jvmTarget = "17"') &&
    !configuredKotlinGradle.includes("VERSION_1_8") &&
    !configuredKotlinGradle.includes('jvmTarget = "1.8"') &&
    !configuredKotlinGradle.includes("isCoreLibraryDesugaringEnabled = false"),
  "Gradle injection enforces Java 17, Kotlin JVM 17, and Kotlin DSL core library desugaring",
);
check(
  injectWireGuardDependency(configuredKotlinGradle, "kotlin") === configuredKotlinGradle,
  "Kotlin Gradle compatibility injection is idempotent",
);

const groovyGradleFixture = `android {\n    compileOptions {\n        sourceCompatibility JavaVersion.VERSION_11\n        targetCompatibility JavaVersion.VERSION_11\n        coreLibraryDesugaringEnabled false\n    }\n    kotlinOptions {\n        jvmTarget = '11'\n    }\n}\n\ndependencies {\n    implementation 'com.wireguard.android:tunnel:1.0.20250531'\n    coreLibraryDesugaring 'com.android.tools:desugar_jdk_libs:2.0.4'\n    implementation 'example:kept:1.0'\n}\n`;
const configuredGroovyGradle = injectWireGuardDependency(groovyGradleFixture, "groovy");
check(
  !configuredGroovyGradle.includes("1.0.20250531") &&
    !configuredGroovyGradle.includes("desugar_jdk_libs:2.0.4") &&
    configuredGroovyGradle.includes(`implementation("${WIREGUARD_DEPENDENCY}")`) &&
    configuredGroovyGradle.includes(`coreLibraryDesugaring("${DESUGAR_DEPENDENCY}")`) &&
    configuredGroovyGradle.includes("implementation 'example:kept:1.0'"),
  "Gradle injection upgrades stale Groovy dependency coordinates without replacing generated content",
);
check(
  configuredGroovyGradle.includes("sourceCompatibility = JavaVersion.VERSION_17") &&
    configuredGroovyGradle.includes("targetCompatibility = JavaVersion.VERSION_17") &&
    configuredGroovyGradle.includes("coreLibraryDesugaringEnabled true") &&
    configuredGroovyGradle.includes('jvmTarget = "17"') &&
    !configuredGroovyGradle.includes("VERSION_11") &&
    !configuredGroovyGradle.includes("coreLibraryDesugaringEnabled false") &&
    injectWireGuardDependency(configuredGroovyGradle, "groovy") === configuredGroovyGradle,
  "Groovy Gradle compatibility injection enforces Java/Kotlin 17 and is idempotent",
);
let missingAndroidBlockRejected = false;
try {
  injectWireGuardDependency('dependencies { implementation("example:kept:1.0") }\n');
} catch {
  missingAndroidBlockRejected = true;
}
check(
  missingAndroidBlockRejected,
  "Gradle compatibility injection fails clearly instead of blindly rewriting an unknown Android block shape",
);

const androidNativeRoot = join(root, "src-tauri", "native", "noland-moonlight");
const androidNativeCmake = readFileSync(join(androidNativeRoot, "CMakeLists.txt"), "utf8");
const androidVideoRenderer = readFileSync(join(androidNativeRoot, "src", "noland_video_renderer_android.c"), "utf8");
const androidAudioRenderer = readFileSync(join(androidNativeRoot, "src", "noland_audio_renderer_android.c"), "utf8");
const androidStreamBridge = readFileSync(join(root, "src-tauri", "src", "moonlight", "platform", "android.rs"), "utf8");
check(
  !tauriBuildScript.includes('.file("native/android/noland_android_runtime_stub.c")') &&
    androidNativeCmake.includes("noland_video_renderer_android.c") &&
    androidNativeCmake.includes("noland_audio_renderer_android.c"),
  "Android links the real Moonlight runtime and media renderers, not the shell stub",
);
check(
  androidVideoRenderer.includes("AMediaCodec_configure") &&
    androidVideoRenderer.includes("AMediaCodec_releaseOutputBuffer") &&
    androidVideoRenderer.includes("LiCompleteVideoFrame(handle, result)"),
  "Android native decoder renders to a surface and completes borrowed core frames",
);
check(
  androidAudioRenderer.includes("opus_multistream_decode") && androidAudioRenderer.includes("slCreateEngine"),
  "Android audio uses real Opus decoding and bounded OpenSL ES playback",
);
check(
  androidStreamBridge.includes("ANativeWindow_fromSurface") &&
    androidStreamBridge.includes("s.token != token") &&
    androidStreamBridge.includes("runtime.detach_surface().await"),
  "Android surface lifecycle rejects stale routes and waits for actor detach before release",
);

const requiredIgnorePatterns = [
  "src-tauri/gen/android/",
  "src-tauri/android/.gradle/",
  "src-tauri/android/**/build/",
  "**/local.properties",
  "*.apk",
  "*.aab",
  "*.apks",
  "*.jks",
  "*.keystore",
  "key.properties",
];
for (const pattern of requiredIgnorePatterns) {
  check(ignoreLines.has(pattern), `.gitignore protects ${pattern}`);
}

const forbiddenConfigTerms = ["signingConfig", "storeFile", "storePassword", "keyAlias", "keyPassword"];
const serializedConfig = JSON.stringify(androidConfig);
for (const term of forbiddenConfigTerms) {
  check(!serializedConfig.includes(term), `Android config contains no embedded ${term}`);
}

const overlayRoot = join(root, "src-tauri", "android");
const requiredOverlayFiles = [
  "app/src/main/AndroidManifest.xml",
  "app/src/main/java/com/noland/connect/stream/MediaCodecVideoDecoder.kt",
  "app/src/main/java/com/noland/connect/stream/NativeStreamBridge.kt",
  "app/src/main/java/com/noland/connect/stream/StreamActivity.kt",
  "app/src/main/java/com/noland/connect/stream/StreamSurfaceView.kt",
  "app/src/main/java/com/noland/connect/stream/StreamTouchController.kt",
  "app/src/main/java/com/noland/connect/stream/StreamControlsOverlay.kt",
  "app/src/main/java/com/noland/connect/stream/StreamKeyboardView.kt",
  "app/src/main/java/com/noland/connect/stream/StreamHardwareInput.kt",
  "app/src/main/java/com/noland/connect/stream/VirtualGamepadView.kt",
  "app/src/main/java/com/noland/connect/vpn/NolandVpnBridge.kt",
  "app/src/main/java/com/noland/connect/vpn/VpnPermissionActivity.kt",
];
for (const relativePath of requiredOverlayFiles) {
  check(existsSync(join(overlayRoot, relativePath)), `Android overlay provides ${relativePath}`);
}

const streamSources = join(overlayRoot, "app", "src", "main", "java", "com", "noland", "connect", "stream");
const streamActivity = readFileSync(join(streamSources, "StreamActivity.kt"), "utf8");
const streamControls = readFileSync(join(streamSources, "StreamControlsOverlay.kt"), "utf8");
const streamBridge = readFileSync(join(streamSources, "NativeStreamBridge.kt"), "utf8");
const frontendPlatform = readFileSync(join(root, "src", "lib", "platform.ts"), "utf8");
const appFrontend = readFileSync(join(root, "src", "app", "App.tsx"), "utf8");
check(
  ["Trackpad", "Direct touch", "Click-to-use", "On-screen controller", "Keyboard", "Return to Noland", "Resume stream"]
    .every((label) => streamControls.includes(`"${label}"`)),
  "Android stream drawer exposes the reference touch modes, controller, keyboard, and navigation",
);
check(
  streamActivity.includes("streamView.touchSink = object") &&
    streamActivity.includes("routeKeyboardGesture(event)") &&
    streamActivity.includes("toolbarSequence"),
  "Android installs the multi-touch state machine and preserves keyboard toolbar touch ownership",
);
check(
  streamActivity.includes("incarnation != 0L && !dashboardLeaving && !closedFromRust") &&
    androidStreamBridge.includes("pub async fn present_stream()"),
  "Android reclaims a retained Activity after dashboard handoff without restarting the session",
);
check(
  ["nativePointer", "nativeButton", "nativeScroll", "nativeText", "nativeController", "nativeSuspendInput", "nativeDashboard"]
    .every((method) => streamBridge.includes(`external fun ${method}`)),
  "Android stream JNI declares pointer, keyboard, controller, suspension, and dashboard callbacks",
);
check(
  frontendPlatform.includes('TAURI_ENV_PLATFORM === "android"') &&
    appFrontend.includes("isMobile && embeddedMoonlightStatus?.runtimeConnected"),
  "Android dashboard exposes connected-stream reentry instead of gating it to iOS",
);

const manifestPath = join(overlayRoot, "app", "src", "main", "AndroidManifest.xml");
const manifest = existsSync(manifestPath) ? readFileSync(manifestPath, "utf8") : "";
check(
  manifest.includes('android:name=".stream.StreamActivity"') &&
    manifest.includes('android:exported="false"'),
  "Android stream activity is private to the application",
);
check(
  manifest.includes('android:name=".vpn.VpnPermissionActivity"') &&
    manifest.includes('android:theme="@android:style/Theme.Translucent.NoTitleBar"'),
  "Android manifest registers the private VPN permission activity",
);
check(
  manifest.includes('android:allowBackup="false"') &&
    manifest.includes('android:fullBackupContent="false"'),
  "Android manifest excludes private tunnel state from platform backups",
);

const decoderPath = join(
  overlayRoot,
  "app",
  "src",
  "main",
  "java",
  "com",
  "noland",
  "connect",
  "stream",
  "MediaCodecVideoDecoder.kt",
);
const decoder = existsSync(decoderPath) ? readFileSync(decoderPath, "utf8") : "";
check(
  decoder.includes("MediaCodec.createDecoderByType") && decoder.includes("releaseOutputBuffer"),
  "Android stream overlay uses a surface-backed MediaCodec decoder",
);

const vpnBridgePath = join(
  overlayRoot,
  "app",
  "src",
  "main",
  "java",
  "com",
  "noland",
  "connect",
  "vpn",
  "NolandVpnBridge.kt",
);
const vpnBridge = existsSync(vpnBridgePath) ? readFileSync(vpnBridgePath, "utf8") : "";
const vpnMethods = [
  "installAndStart",
  "reconnect",
  "stop",
  "remove",
  "status",
  "runtime",
  "configuration",
  "update",
];
check(
  vpnBridge.includes("package com.noland.connect.vpn") &&
    vpnBridge.includes("object NolandVpnBridge") &&
    vpnMethods.every((method) =>
      new RegExp(`@JvmStatic\\s+fun ${method}\\(context: Context, requestJson: String\\): String`, "u")
        .test(vpnBridge),
    ),
  "Android VPN overlay exposes every required static Context/JSON bridge method",
);
check(
  vpnBridge.includes("import androidx.annotation.Keep") &&
    /@Keep\s+object NolandVpnBridge/u.test(vpnBridge),
  "Android VPN bridge JNI class and static methods are retained in release builds",
);
check(
  vpnBridge.includes("GoBackend") &&
    vpnBridge.includes("Config.parse") &&
    vpnBridge.includes(": Tunnel") &&
    vpnBridge.includes('TUNNEL_NAME = "noland"'),
  "Android VPN bridge uses the pinned tunnel library with one noland tunnel",
);
const vpnPermissionActivityPath = join(
  overlayRoot,
  "app",
  "src",
  "main",
  "java",
  "com",
  "noland",
  "connect",
  "vpn",
  "VpnPermissionActivity.kt",
);
const vpnPermissionActivity = existsSync(vpnPermissionActivityPath)
  ? readFileSync(vpnPermissionActivityPath, "utf8")
  : "";
check(
  vpnBridge.includes("VpnService.prepare(context)") &&
    vpnBridge.includes("VpnPermissionActivity.request(context)") &&
    vpnBridge.includes('"permission_required"'),
  "Android VPN bridge waits for the system permission result before continuing",
);
check(
  vpnPermissionActivity.includes("class VpnPermissionActivity : Activity()") &&
    vpnPermissionActivity.includes("startActivityForResult") &&
    vpnPermissionActivity.includes("onActivityResult") &&
    vpnPermissionActivity.includes("CountDownLatch"),
  "Android VPN permission activity prompts the user and reports approval to the waiting tunnel operation",
);
check(
  vpnBridge.includes("Context.MODE_PRIVATE") &&
    ["configuration", "configuration_reference", "launch_id", "config_fingerprint"].every(
      (key) => vpnBridge.includes(`"${key}"`),
    ),
  "Android VPN bridge durably stores configuration, reference, launch, and fingerprint in private preferences",
);
const runtimeFields = [
  "ok",
  "error",
  "active",
  "launchId",
  "configFingerprint",
  "peerPublicKey",
  "endpoint",
  "mtu",
  "latestHandshakeAgeSecs",
  "rxBytes",
  "txBytes",
];
check(
  runtimeFields.every((field) => vpnBridge.includes(`"${field}"`)) &&
    vpnBridge.includes("getStatistics(NolandTunnel)") &&
    vpnBridge.includes("latestHandshakeEpochMillis"),
  "Android VPN runtime response uses camelCase native fields and GoBackend statistics",
);

const forbiddenArtifactNames = new Set(["key.properties", "local.properties"]);
const forbiddenArtifactExtensions = new Set([".apk", ".aab", ".apks", ".jks", ".keystore"]);
const forbiddenArtifacts = [];

function inspectOverlay(directory) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isSymbolicLink()) continue;
    if (entry.isDirectory()) {
      if (entry.name !== "build" && entry.name !== ".gradle") inspectOverlay(path);
      continue;
    }
    if (!entry.isFile() || !lstatSync(path).isFile()) continue;
    const lowerName = entry.name.toLowerCase();
    const extension = lowerName.slice(lowerName.lastIndexOf("."));
    if (forbiddenArtifactNames.has(lowerName) || forbiddenArtifactExtensions.has(extension)) {
      forbiddenArtifacts.push(relative(root, path));
    }
  }
}

if (existsSync(overlayRoot)) inspectOverlay(overlayRoot);
check(
  forbiddenArtifacts.length === 0,
  forbiddenArtifacts.length === 0
    ? "Maintained Android overlay contains no local, signing, or package artifacts"
    : `Maintained Android overlay contains forbidden artifacts: ${forbiddenArtifacts.join(", ")}`,
);

if (failures.length > 0) {
  console.error(`Android scaffold check failed (${failures.length} issue${failures.length === 1 ? "" : "s"}):`);
  for (const failure of failures) console.error(`- ${failure}`);
  process.exit(1);
}

console.log(`Android scaffold check passed (${checks.length} checks; no Android SDK required).`);
