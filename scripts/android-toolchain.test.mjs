import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  DESUGAR_DEPENDENCY,
  injectWireGuardDependency,
  WIREGUARD_DEPENDENCY,
} from "./configure-android-project.mjs";

const workflow = readFileSync(new URL("../.github/workflows/android.yml", import.meta.url), "utf8");
const step = workflow.split("      - name: Select the pinned Android native toolchain\n")[1]?.split("      - uses:")[0];
const script = step?.split("        run: |\n")[1]?.replace(/^ {10}/gmu, "");
assert.ok(script, "The Android workflow must contain the pinned toolchain selection step");

function fixture(t, { pinned = true } = {}) {
  const directory = mkdtempSync(join(tmpdir(), "noland android toolchain "));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const sdk = join(directory, "sdk");
  const ndks = join(sdk, "ndk");
  const version = "29.0.14206865";
  for (const installed of ["27.3.13750724", "30.0.16248370", ...(pinned ? [version] : [])]) {
    const ndk = join(ndks, installed);
    mkdirSync(join(ndk, "build", "cmake"), { recursive: true });
    writeFileSync(join(ndk, "build", "cmake", "android.toolchain.cmake"), "");
    writeFileSync(join(ndk, "source.properties"), `Pkg.Revision = ${installed}\n`);
  }
  const cmake = join(sdk, "cmake", "3.22.1", "bin");
  mkdirSync(cmake, { recursive: true });
  for (const tool of ["cmake", "ninja"]) {
    const executable = join(cmake, tool);
    writeFileSync(executable, "#!/bin/sh\nprintf '%s\\n' 'native tool fixture'\n");
    chmodSync(executable, 0o755);
  }
  const runnerTemp = join(directory, "runner temp");
  mkdirSync(runnerTemp);
  const envFile = join(directory, "github env");
  const pathFile = join(directory, "github path");
  writeFileSync(envFile, "");
  writeFileSync(pathFile, "");
  const run = () => spawnSync("bash", ["-c", script], {
    encoding: "utf8",
    timeout: 10_000,
    env: {
      ...process.env,
      ANDROID_HOME: sdk,
      ANDROID_NDK_VERSION: version,
      ANDROID_CMAKE_VERSION: "3.22.1",
      RUNNER_TEMP: runnerTemp,
      GITHUB_ENV: envFile,
      GITHUB_PATH: pathFile,
    },
  });
  return { sdk, ndks, version, cmake, runnerTemp, envFile, pathFile, run };
}

// Rendered from the CLI 2.12.4 app template (no optional plugins/dependencies):
// https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.4/crates/tauri-cli/templates/mobile/android/app/build.gradle.kts
const cli2124Gradle = `import java.util.Properties
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

android {
    compileSdk = 37
    namespace = "com.example.app"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.example.app"
        minSdk = 24
        targetSdk = 37
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {
                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            optimization {
               enable = true
            }
            proguardFiles(
                *fileTree(".") {
                  include("**/*.pro")
                  exclude("build/**")
                }.files.toTypedArray()
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    buildFeatures {
        buildConfig = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = file("tauri.build.gradle.kts"))
`;

function assertCompatibilityOverlay(source, dialect = "kotlin") {
  const configured = injectWireGuardDependency(source, dialect);
  assert.match(configured, /sourceCompatibility = JavaVersion\.VERSION_17/u);
  assert.match(configured, /targetCompatibility = JavaVersion\.VERSION_17/u);
  assert.doesNotMatch(configured, /VERSION_(?:1_8|11)\b|JVM_(?:1_8|11)\b|jvmTarget\s*=\s*["'](?:1\.8|11)["']/u);
  assert.match(configured, dialect === "kotlin"
    ? /isCoreLibraryDesugaringEnabled = true/u
    : /coreLibraryDesugaringEnabled true/u);
  for (const dependency of [WIREGUARD_DEPENDENCY, DESUGAR_DEPENDENCY]) {
    assert.equal(configured.split(dependency).length - 1, 1);
  }
  assert.ok(configured.includes(`implementation("${WIREGUARD_DEPENDENCY}")`));
  assert.ok(configured.includes(`coreLibraryDesugaring("${DESUGAR_DEPENDENCY}")`));
  assert.equal(injectWireGuardDependency(configured, dialect), configured);
  return configured;
}

test("CLI 2.12.4 compilerOptions and Java both target 17 without legacy kotlinOptions", () => {
  const configured = assertCompatibilityOverlay(cli2124Gradle);
  assert.match(configured, /kotlin \{\s+compilerOptions \{\s+jvmTarget = JvmTarget\.JVM_17/u);
  assert.doesNotMatch(configured, /kotlinOptions/u);
  // Everything outside the targets and managed overlays must survive unchanged.
  const withoutOverlay = configured
    .replace(/^ *\/\/ Noland Java 17 compatibility start\n[\s\S]*?^ *\/\/ Noland Java 17 compatibility end\n/gmu, "")
    .replace(/^ *\/\/ Noland (?:native WireGuard tunnel|Java 17 core library desugaring) dependency\n[^\n]*\n/gmu, "");
  assert.equal(withoutOverlay, cli2124Gradle
    .replaceAll("JavaVersion.VERSION_1_8", "JavaVersion.VERSION_17")
    .replace("JvmTarget.JVM_1_8", "JvmTarget.JVM_17"));
});

test("modern enum assignments and setters retain their syntax and qualification", () => {
  for (const target of [
    "jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_11",
    "jvmTarget.set(JvmTarget.JVM_1_8)",
    "jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_11)",
  ]) {
    const configured = assertCompatibilityOverlay(cli2124Gradle.replace("jvmTarget = JvmTarget.JVM_1_8", target));
    assert.ok(configured.includes(target.replace(/JVM_(?:1_8|11)/u, "JVM_17")));
    assert.doesNotMatch(configured, /kotlinOptions/u);
  }
});

test("reapplying an old overlay to CLI 2.12.4 removes stale managed kotlinOptions", () => {
  const previouslyConfigured = cli2124Gradle.replace("    buildFeatures {", `    // Noland Java 17 compatibility start
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
        isCoreLibraryDesugaringEnabled = true
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    // Noland Java 17 compatibility end
    buildFeatures {`);
  assert.equal(assertCompatibilityOverlay(previouslyConfigured), injectWireGuardDependency(cli2124Gradle));
});

test("legacy Kotlin and Groovy templates keep JVM 17 and upgrade pinned overlay dependencies", () => {
  for (const dialect of ["kotlin", "groovy"]) {
    const source = `plugins { id("com.android.application") }
android {
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
        ${dialect === "kotlin" ? "isCoreLibraryDesugaringEnabled = false" : "coreLibraryDesugaringEnabled false"}
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
}
dependencies {
    implementation("example:kept:1.0")
    implementation("com.wireguard.android:tunnel:1.0.20250531")
    coreLibraryDesugaring("com.android.tools:desugar_jdk_libs:2.0.4")
}
`;
    const configured = assertCompatibilityOverlay(source, dialect);
    assert.match(configured, /kotlinOptions \{\s+jvmTarget = "17"/u);
    assert.ok(configured.includes('implementation("example:kept:1.0")'));
    assert.doesNotMatch(configured, /1\.0\.20250531|desugar_jdk_libs:2\.0\.4|(?:DesugaringEnabled =|DesugaringEnabled) false/u);
    const withoutKotlinOptions = source.replace(/    kotlinOptions \{\n[^\n]*\n    \}\n/u, "");
    assert.match(assertCompatibilityOverlay(withoutKotlinOptions, dialect), /kotlinOptions \{\s+jvmTarget = "17"/u);
  }
});

test("CI isolates the pinned NDK before Tauri discovery and exports a consistent toolchain", (t) => {
  const f = fixture(t);
  const result = f.run();
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(readdirSync(f.ndks), [f.version]);
  for (const version of ["27.3.13750724", "30.0.16248370"]) {
    assert.ok(existsSync(join(f.runnerTemp, "noland-unused-ndks", version)));
  }
  const exports = readFileSync(f.envFile, "utf8");
  assert.ok(exports.includes(`NDK_HOME=${join(f.ndks, f.version)}\n`));
  assert.ok(exports.includes(`ANDROID_NDK_HOME=${join(f.ndks, f.version)}\n`));
  assert.ok(exports.includes(`CMAKE=${join(f.cmake, "cmake")}\n`));
  assert.equal(readFileSync(f.pathFile, "utf8"), `${f.cmake}\n`);
  assert.equal(f.run().status, 0, "The selection step must be safe to run again");
});

test("CI fails before moving NDKs or exporting paths if the pinned NDK is missing", (t) => {
  const f = fixture(t, { pinned: false });
  assert.notEqual(f.run().status, 0);
  assert.deepEqual(readdirSync(f.ndks).sort(), ["27.3.13750724", "30.0.16248370"]);
  assert.equal(readFileSync(f.envFile, "utf8"), "");
  assert.equal(readFileSync(f.pathFile, "utf8"), "");
});
