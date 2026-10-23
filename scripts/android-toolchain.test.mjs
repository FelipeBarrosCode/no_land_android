import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

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
