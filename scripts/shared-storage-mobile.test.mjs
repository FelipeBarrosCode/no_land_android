import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test, { beforeEach } from "node:test";

// Exercise the real mobile adapter without installed web dependencies, native
// builds, credentials, SSH, or provider access. Script CI discovers this suite.
const backendUrl = new URL("../src/lib/backend.ts", import.meta.url).href;
const mocks = {
  "@tauri-apps/api/event": "export const listen = (...args) => globalThis.__storageListen(...args);",
  "./tauri": "export const invokeSafe = (...args) => globalThis.__storageInvoke(...args);",
  "./i18n": "export const getActiveLocale = () => 'en-US'; export const translate = key => key;",
  "../features/feedback/model": "export const offerMachineIdentity = value => value; export const parseFeedbackProgram = value => value; export const feedbackOfferMachineId = () => null;",
  "../features/feedback/data": "export const invalidateFeedbackData = () => {};",
};
registerHooks({
  resolve(specifier, context, nextResolve) {
    if (context.parentURL === backendUrl && Object.hasOwn(mocks, specifier)) {
      return { url: `data:text/javascript,${encodeURIComponent(mocks[specifier])}`, shortCircuit: true };
    }
    return nextResolve(specifier, context);
  },
});
const backend = await import(backendUrl);
let calls;
let listeners;
let response;

beforeEach(() => {
  calls = [];
  listeners = new Map();
  response = undefined;
  globalThis.__storageInvoke = async (command, args) => {
    calls.push([command, args]);
    return response;
  };
  globalThis.__storageListen = async (event, callback) => {
    listeners.set(event, callback);
    return () => listeners.delete(event);
  };
});

test("mobile tree reads forward entries without starting backup, restore, or reconciliation", async () => {
  response = [{ path: "/apps/steam:123", parentPath: "/apps", name: "Game", isDir: false }];
  assert.equal(await backend.listInstanceSharedStorageObjects(17), response);
  assert.equal(await backend.listInstanceExportableStorageObjects(17), response);
  assert.deepEqual(calls, [
    ["list_instance_shared_storage_objects", { instanceId: 17 }],
    ["list_instance_exportable_storage_objects", { instanceId: 17 }],
  ]);
});

test("snapshot reads preserve opaque scope, freshness, target and explicit refresh intent", async () => {
  response = {
    entries: [],
    scope: "opaque:repository/account/instance/binding",
    observedAt: null,
    freshness: "unavailable",
  };
  assert.equal(await backend.getInstanceStorageBrowseSnapshot(17, "cloud"), response);
  response = { ...response, observedAt: "2026-11-15T00:00:00Z", freshness: "fresh" };
  assert.equal(await backend.getInstanceStorageBrowseSnapshot(23, "remote_pc", true), response);
  assert.deepEqual(calls, [
    ["get_instance_storage_browse_snapshot", { instanceId: 17, target: "cloud", refresh: false }],
    ["get_instance_storage_browse_snapshot", { instanceId: 23, target: "remote_pc", refresh: true }],
  ]);
});

test("a failed cached snapshot read does not fall back to a network-backed legacy tree", async () => {
  const failure = new Error("Disposable cache unavailable");
  globalThis.__storageInvoke = async (command, args) => {
    calls.push([command, args]);
    throw failure;
  };
  await assert.rejects(backend.getInstanceStorageBrowseSnapshot(17, "cloud"), error => error === failure);
  assert.deepEqual(calls, [
    ["get_instance_storage_browse_snapshot", { instanceId: 17, target: "cloud", refresh: false }],
  ]);
});

test("selected restore keeps literal application paths and explicit instance identity", async () => {
  const selectedPaths = ["/apps/steam:123", "/apps/O'Brien; $(literal)/save.dat"];
  response = "queued";
  assert.equal(await backend.syncInstanceFromSharedStorageSelected(23, selectedPaths), "queued");
  assert.deepEqual(calls, [[
    "sync_instance_from_shared_storage_selected",
    { instanceId: 23, payload: { selectedPaths } },
  ]]);
});

test("selected backup leaves automatic scheduling to the agent", async () => {
  const selectedPaths = ["/apps/steam:123"];
  await backend.saveInstanceToSharedStorageSelected(31, selectedPaths);
  assert.deepEqual(calls, [
    ["save_instance_to_shared_storage_selected", { instanceId: 31, payload: { selectedPaths } }],
  ]);
});

test("launch-ready progress never synthesizes final restore completion", async () => {
  const progress = [];
  const completed = [];
  const stopProgress = await backend.subscribeSharedStorageProgress(value => progress.push(value));
  const stopCompleted = await backend.subscribeSharedStorageRestoreCompleted(value => completed.push(value));
  const sample = {
    instanceId: 17,
    kind: "restore",
    phase: "APPLYING",
    message: "READY_TO_LAUNCH",
    completedBytes: 64,
    totalBytes: 128,
  };
  listeners.get("shared-storage:progress")({ payload: sample });
  assert.deepEqual(progress, [sample]);
  assert.deepEqual(completed, []);
  const final = { instanceId: 17, restoredAppIds: ["steam:123"] };
  listeners.get("shared-storage:restore-completed")({ payload: final });
  assert.deepEqual(completed, [final]);
  stopProgress();
  stopCompleted();
  assert.equal(listeners.size, 0, "closed mobile views must release native event listeners");
  assert.deepEqual(calls, []);
});

test("native read failures remain visible instead of becoming empty successful trees", async () => {
  const failure = { message: "Agent unavailable", details: "Connection lost" };
  globalThis.__storageInvoke = async () => { throw failure; };
  await assert.rejects(backend.listInstanceSharedStorageObjects(17), error => error === failure);
  await assert.rejects(backend.listInstanceExportableStorageObjects(17), error => error === failure);
});

test("explicit index refresh preserves zero counters and false loss-state results", async () => {
  response = {
    apps_reconciled: 0,
    paths_reconciled: 7,
    excluded_flags_cleared: 0,
    processed_events: 11,
    loss_state_cleared: false,
  };
  assert.deepEqual(await backend.refreshStateAgentIndex(41), {
    appsReconciled: 0,
    pathsReconciled: 7,
    excludedFlagsCleared: 0,
    processedEvents: 11,
    lossStateCleared: false,
  });
  assert.deepEqual(calls, [["refresh_state_agent_index", { instanceId: 41 }]]);
});

test("cancelling preserves operation identity and does not issue another operation", async () => {
  response = "cancel requested";
  assert.equal(await backend.cancelSharedStorageOperation(53, "restore-53"), response);
  assert.deepEqual(calls, [["cancel_shared_storage_operation", { instanceId: 53, operationId: "restore-53" }]]);
});

test("legacy cancellation remains optional without inventing an operation identity", async () => {
  await backend.cancelSharedStorageOperation(53);
  assert.deepEqual(calls, [["cancel_shared_storage_operation", { instanceId: 53, operationId: undefined }]]);
});
