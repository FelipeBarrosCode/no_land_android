import { existsSync, lstatSync, readFileSync, readdirSync } from "node:fs";
import { basename, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

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

const expectedResources = [
  "../vm-cloud-mic-agent/Cargo.toml",
  "../vm-cloud-mic-agent/src/**/*",
  "../network-agent/Cargo.toml",
  "../network-agent/Cargo.lock",
  "../network-agent/src/**/*",
  "../network-contracts/Cargo.toml",
  "../network-contracts/Cargo.lock",
  "../network-contracts/src/**/*",
  "../state-agent/Cargo.toml",
  "../state-agent/Cargo.lock",
  "../state-agent/rust-toolchain.toml",
  "../state-agent/crates/**/*",
  "../state-agent/scripts/**/*",
  "../state-agent/systemd/**/*",
];
const resources = androidConfig.bundle?.resources;
check(
  Array.isArray(resources) &&
    resources.length === expectedResources.length &&
    expectedResources.every((resource) => resources.includes(resource)),
  "Android config preserves the shared remote-agent resources",
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
  !/child_process|\bgradle\b|ANDROID_HOME|ANDROID_SDK_ROOT/u.test(configureScript),
  "Configure script does not introduce an implicit SDK operation",
);

const androidRuntimeStubPath = join(
  root,
  "src-tauri",
  "native",
  "android",
  "noland_android_runtime_stub.c",
);
const androidRuntimeStub = existsSync(androidRuntimeStubPath)
  ? readFileSync(androidRuntimeStubPath, "utf8")
  : "";
check(Boolean(androidRuntimeStub), "Android link-safe Moonlight runtime ABI adapter exists");
check(
  androidRuntimeStub.includes("nl_runtime_create") &&
    androidRuntimeStub.includes("NL_RESULT_INVALID_STATE") &&
    androidRuntimeStub.includes('return "android-shell"'),
  "Android runtime adapter stays idle and rejects unsupported streaming",
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
];
for (const relativePath of requiredOverlayFiles) {
  check(existsSync(join(overlayRoot, relativePath)), `Android overlay provides ${relativePath}`);
}

const manifestPath = join(overlayRoot, "app", "src", "main", "AndroidManifest.xml");
const manifest = existsSync(manifestPath) ? readFileSync(manifestPath, "utf8") : "";
check(
  manifest.includes('android:name=".stream.StreamActivity"') &&
    manifest.includes('android:exported="false"'),
  "Android stream activity is private to the application",
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
