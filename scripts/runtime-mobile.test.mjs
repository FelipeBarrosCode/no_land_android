import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import ts from "typescript";

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
const source = read("src/features/runtime/runtimeModel.ts");
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText;
const { createRuntimePoller, isRemoteExecutablePath, isRuntimeInstalling, parseRuntimePlan, runtimeError } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString("base64")}`);

test("only active installation phases disable competing operations", () => {
  for (const phase of ["checking", "downloading", "verifying", "extracting", "validating"]) assert.equal(isRuntimeInstalling(phase), true);
  for (const phase of [undefined, "absent", "ready", "failed", "cancelled"]) assert.equal(isRuntimeInstalling(phase), false);
});

test("remote paths stay literal and reject device picker URIs/control characters", () => {
  assert.equal(isRemoteExecutablePath("/home/user/O'Brien; $(touch owned) App.AppImage"), true);
  for (const path of ["", "/", "~/Downloads/App", "file:///tmp/App", "content://downloads/App", "/tmp/a\nb", "/tmp/a\0b"]) assert.equal(isRemoteExecutablePath(path), false);
});

test("download consent requires real measured sizes and an artifact digest", () => {
  const plan = { requiredBytes: 1000, manifest: { id: "v1", sha256: "ab".repeat(32), downloadBytes: 100, extractedBytes: 500 } };
  assert.equal(parseRuntimePlan(plan), plan);
  for (const value of [null, {}, { ...plan, requiredBytes: NaN }, { ...plan, requiredBytes: 10 }, { ...plan, manifest: { ...plan.manifest, id: "../escape" } }, { ...plan, manifest: { ...plan.manifest, sha256: "missing" } }, { ...plan, manifest: { ...plan.manifest, downloadBytes: -1 } }]) assert.equal(parseRuntimePlan(value), null);
});

test("structured SSH errors retain actionable message and details", () => {
  assert.equal(runtimeError({ message: "SSH failed", details: "Connection lost" }), "SSH failed: Connection lost");
  assert.equal(runtimeError(new Error("Disconnected")), "Disconnected");
});

test("background/closed views do not start SSH polling and foreground can resume", async () => {
  let visible = false; let calls = 0; const values = [];
  const poller = createRuntimePoller({ fetch: async () => ++calls, shouldPoll: () => visible, onValue: (v) => values.push(v), onError: assert.fail });
  await poller.tick(); assert.equal(calls, 0);
  visible = true; await poller.tick(); assert.deepEqual(values, [1]);
  poller.stop(); await poller.tick(); assert.equal(calls, 1);
});

test("poll requests do not overlap and stopped views ignore late responses", async () => {
  let resolve; let calls = 0; const values = [];
  const poller = createRuntimePoller({ fetch: () => { ++calls; return new Promise((done) => { resolve = done; }); }, shouldPoll: () => true, onValue: (v) => values.push(v), onError: assert.fail });
  const pending = poller.tick(); await poller.tick(); assert.equal(calls, 1);
  poller.stop(); resolve("stale"); await pending; assert.deepEqual(values, []);
});

test("stopped views ignore late errors without cancelling authorized remote work", async () => {
  let reject; const errors = [];
  const poller = createRuntimePoller({ fetch: () => new Promise((_, fail) => { reject = fail; }), shouldPoll: () => true, onValue: assert.fail, onError: (e) => errors.push(e) });
  const pending = poller.tick(); poller.stop(); reject(new Error("late")); await pending;
  assert.deepEqual(errors, []);
});

test("runtime commands use the existing in-process mobile SSH abstraction", () => {
  const manager = read("src-tauri/src/services/runtime_manager.rs");
  for (const command of ["get_instance_runtime_status", "instance_runtime_action"]) assert.ok(read("src-tauri/src/lib.rs").includes(command));
  assert.ok(manager.includes("remote.ssh_with_stdin"));
  assert.ok(manager.includes("Sha256::digest(&bytes)"));
  assert.ok(manager.includes('url.scheme() != "https"'));
  assert.ok(!manager.includes("std::process"));
  assert.ok(!manager.includes("portable_pty"));
  assert.ok(read("src-tauri/src/commands/runtime.rs").includes('"--expected-sha256"'));
});

test("explicit runtime launches do not fall back to native/Steam or inherit their session", () => {
  const launch = read("src-tauri/src/services/launch_library.rs");
  assert.ok(launch.includes("LaunchPlan::Steam(_) | LaunchPlan::Runtime(_)"));
  assert.ok(launch.includes('"runtime:ubuntu24-x86_64"'));
  assert.ok(launch.includes("runtime_route_keeps_app_ids_literal_and_does_not_inherit_parent_session"));
});

test("runtime UI copy is complete across all nine locale bundles", () => {
  const en = JSON.parse(read("src/locales/en-US.json"));
  const keys = Object.keys(en).filter((key) => key.startsWith("runtime."));
  assert.equal(keys.length, 19);
  for (const locale of ["en-US", "pt-BR", "es", "fr", "de", "it", "ja", "ko", "zh-CN"]) {
    const messages = JSON.parse(read(`src/locales/${locale}.json`));
    for (const key of keys) {
      assert.ok(messages[key]?.trim(), `${locale}: ${key}`);
      assert.deepEqual(messages[key].match(/\{\w+\}/g), en[key].match(/\{\w+\}/g));
    }
  }
});
