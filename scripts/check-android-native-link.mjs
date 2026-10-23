import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const target = process.argv[2];
const triple = { aarch64: "aarch64-linux-android", x86_64: "x86_64-linux-android" }[target];
assert.ok(triple, "Usage: node scripts/check-android-native-link.mjs <aarch64|x86_64>");
const ndk = process.env.ANDROID_NDK_HOME || process.env.NDK_HOME;
assert.ok(ndk, "Set ANDROID_NDK_HOME or NDK_HOME to the Android NDK");
const prebuilt = join(ndk, "toolchains", "llvm", "prebuilt");
const host = readdirSync(prebuilt).find((name) => existsSync(join(prebuilt, name, "bin", "clang")));
assert.ok(host, "The Android NDK must include a host Clang toolchain");
const compiler = join(prebuilt, host, "bin", `${triple}24-clang`);
const sources = fileURLToPath(new URL("../src-tauri/native/moonlight-common-c/nanors/deps/obl/", import.meta.url));
const directory = mkdtempSync(join(tmpdir(), "noland-android-native-link-"));

function run(args) {
  const result = spawnSync(compiler, args, { encoding: "utf8", timeout: 60_000 });
  assert.ifError(result.error);
  return result;
}

try {
  const query = run(["--print-libgcc-file-name"]);
  assert.equal(query.status, 0, query.stderr);
  const runtime = query.stdout.trim();
  assert.ok(existsSync(runtime), `Compiler runtime archive not found: ${runtime}`);
  const args = [
    "-shared", "-fPIC", "-nodefaultlibs", "-Wl,--no-undefined",
    join(sources, "oblas_lite.c"), join(sources, "oblas_common.c"),
  ];
  if (target === "x86_64") {
    // A normal Clang link implicitly adds compiler-rt and would conceal the bug.
    const withoutRuntime = run([...args, "-lc", "-o", join(directory, "without-runtime.so")]);
    assert.notEqual(withoutRuntime.status, 0, "The regression control must require the compiler runtime");
    assert.match(withoutRuntime.stderr, /__cpu_model|__cpu_features2/u);
    args.push(runtime);
  }
  const linked = run([...args, "-lc", "-o", join(directory, "native-link.so")]);
  assert.equal(linked.status, 0, linked.stderr);
  console.log(`Android ${target} native CPU-dispatch link passed with Rust-style -nodefaultlibs.`);
} finally {
  rmSync(directory, { recursive: true, force: true });
}
