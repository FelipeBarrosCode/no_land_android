import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// Structural integration guards only: these do not compile or launch a mobile
// app, read persisted credentials, or replace the shared Rust behavioral tests.
const read = path => readFileSync(new URL(`../src-tauri/src/${path}`, import.meta.url), "utf8");
const browse = read("services/shared_storage/browse.rs");
const runtime = read("services/shared_storage/agent_runtime.rs");
const functionSource = (source, name) => {
  const header = new RegExp(`(?:^|\\n)[ \\t]*(?:pub(?:\\([^)]*\\))?\\s+)?(?:async\\s+)?fn ${name}\\(`).exec(source);
  assert.ok(header, name);
  const tail = source.slice(header.index + header[0].length);
  const next = /\n[ \t]*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn \w+\(/.exec(tail);
  return tail.slice(0, next?.index);
};

test("mobile entry point registers the exact four-field snapshot command without desktop recovery", () => {
  assert.match(read("lib.rs"), /\bget_instance_storage_browse_snapshot,/);
  const command = functionSource(read("commands/mod.rs"), "get_instance_storage_browse_snapshot");
  assert.match(command, /refresh\.unwrap_or\(false\)/);
  assert.match(command, /browse::get_snapshot/);
  const dto = read("models/app_state.rs").split("pub struct SharedStorageBrowseSnapshot {")[1].split("\n}")[0];
  assert.deepEqual([...dto.matchAll(/pub (\w+):/g)].map(match => match[1]), ["entries", "scope", "observed_at", "freshness"]);
  assert.doesNotMatch(read("services/shared_storage/mod.rs"), /lock_recovery/);
});

test("cache-only snapshots do not enter any network, renewal or deployment path", () => {
  assert.match(browse, /let frozen = refresh\s*\.then/);
  const cacheOnly = browse.split("if !refresh {")[1].split("let frozen = frozen.expect")[0];
  assert.match(cacheOnly, /cache\.peek\(&scope\)/);
  assert.match(cacheOnly, /network_requests = 0u64/);
  assert.doesNotMatch(cacheOnly, /load_frozen|resolve_frozen_remote|mint_ephemeral|ensure_state_agent|ensure_rclone|refresh_oauth/);
});

test("native cache bounds charged capacity and keeps in-flight rows stale", () => {
  const cache = read("services/shared_storage/browse_cache.rs");
  assert.match(cache, /MAX_ROWS: usize = 10_000/);
  assert.match(cache, /MAX_SNAPSHOT_BYTES: usize = 8 \* 1024 \* 1024/);
  assert.match(cache, /MAX_CACHE_BYTES: usize = 16 \* 1024 \* 1024/);
  assert.match(cache, /CACHE_BOOKKEEPING_RESERVE: usize = 16 \* 1024/);
  assert.match(cache, /entries\.capacity\(\)/);
  assert.match(cache, /entry\.path\.capacity\(\)/);
  assert.match(cache, /type SharedOutcome = Result<Arc<CachedValue>, String>/);
  assert.match(cache, /entry\.flight\.is_none\(\)/);
  assert.match(cache, /desktop_browse_inflight_refresh_marks_even_recent_rows_cached/);
});

test("native metadata uses authenticated shared providers and independent catalog/pending reads", () => {
  const metadata = functionSource(browse, "load_provider_metadata");
  assert.match(metadata, /tokio::try_join!/);
  assert.match(metadata, /noland_storage::load_catalog\(storage, master\)/);
  assert.match(metadata, /noland_storage::pending_deletions\(storage, master\)/);
  assert.doesNotMatch(metadata, /\b(?:ensure_root|StartDeletion|StartBackup|sync)\b/);
  assert.match(browse, /write_guarded_ephemeral_session/);
  assert.match(browse, /builder\.mode\(0o700\)/);
});

test("browse freezes keys and profile fields from memory without invoking secure storage", () => {
  const profiles = read("services/shared_storage/provider_profiles.rs");
  const key = functionSource(profiles, "repository_key_from_state");
  assert.match(key, /state\.shared_storage_credentials/);
  assert.match(key, /bytes\.fill\(0\)/);
  assert.doesNotMatch(key, /\.await|load_state|Keychain|keyring/);
  const profile = functionSource(read("services/shared_storage/shared_storage_manager.rs"), "profile_from_state");
  assert.match(profile, /state\.shared_storage_credentials\.profiles/);
  assert.doesNotMatch(profile, /\.await|load_state|retrieve_credentials|refresh_oauth_if_needed\(/);
});

test("read-only agent browsing checks compatibility but never updates or changes transport", () => {
  const call = functionSource(runtime, "call_agent_browse");
  assert.match(call, /matches!\(method, "ListApps" \| "ListCloudCatalog"\)/);
  assert.match(call, /access\s*\.try_read\(\)/);
  assert.match(call, /call_agent_raw_unlocked\(remote, "GetHealth"/);
  assert.match(call, /require_browse_protocol\(&health\)/);
  assert.doesNotMatch(call, /ensure_state_agent\(|ensure_rclone|find_state_agent_source_dir|prebuilt_deployment_matches/);
  assert.match(runtime, /"ListApps" \| "ListCloudCatalog" => \(45, 60\)/);
});

test("deployment and reads share endpoint guards without recursively taking a read lock", () => {
  assert.match(functionSource(runtime, "ensure_state_agent"), /access\.write\(\)\.await/);
  assert.match(functionSource(runtime, "call_agent_raw"), /access\.read\(\)\.await/);
  for (const name of ["ensure_state_agent_exclusive", "deployment_has_active_work", "wait_for_bootstrap"]) {
    const body = functionSource(runtime, name);
    assert.match(body, /call_agent_raw_unlocked\(/, name);
    assert.doesNotMatch(body, /\bprobe_agent\(|\bcall_agent_raw\(/, name);
  }
});

test("only the explicit helper command installs the prerequisite and releases the settings read guard first", () => {
  assert.match(read("lib.rs"), /\bprepare_instance_storage_helper,/);
  for (const name of ["get_instance_storage_browse_snapshot", "prepare_instance_storage_helper"]) {
    assert.match(functionSource(read("commands/mod.rs"), name), /map_err\(crate::services::shared_storage::browse::frontend_error\)/);
  }
  const setup = functionSource(browse, "prepare_helper");
  assert.match(setup, /InstanceLifecycleService::begin_action/);
  assert.match(setup, /BrowseMutationGuard::new/);
  assert.match(setup, /drop\(configuration\);[\s\S]*ensure_rclone_installed\(&remote\)/);
  assert.doesNotMatch(setup, /ensure_state_agent|StartBackup|StartRestore|StartDeletion/);
  for (const name of ["get_snapshot", "load_frozen"]) assert.doesNotMatch(functionSource(browse, name), /prepare_helper/);
  const lifecycle = read("services/instance_lifecycle.rs");
  assert.match(lifecycle, /impl Drop for InstanceActionGuard/);
  assert.match(functionSource(lifecycle, "begin_action"), /Self::acquire_lock\(instance_id, action\)\.await/);
});

test("helper presence probing is boolean-only and limited to failed cloud reads", () => {
  const call = functionSource(runtime, "call_agent_browse");
  assert.match(call, /match result \{\s*Ok\(value\) => Ok\(value\),\s*Err\(_\) =>/);
  assert.ok(call.indexOf("browse_helper_available(remote)") > call.indexOf("Err(_)"));
  assert.match(call, /method == "ListCloudCatalog"/);
  assert.match(functionSource(runtime, "parse_browse_helper_presence"), /as_bool/);
  assert.match(functionSource(browse, "frontend_error"), /details: None/);
});

test("legacy cloud/PC lists share complete-load coalescing instead of provisioning", () => {
  const lifecycle = read("services/instance_lifecycle.rs");
  for (const name of ["list_shared_storage_objects", "list_instance_exportable_objects"]) {
    const body = functionSource(lifecycle, name);
    assert.match(body, /shared_storage::browse::list\(/);
    assert.doesNotMatch(body, /build_remote_exec|VastApiClient|ensure_state_agent/);
  }
});

test("mutations invalidate browse state at admission and completion", () => {
  const handoff = read("services/shared_storage/agent_handoff.rs");
  for (const name of ["start_agent_backup", "start_agent_restore", "start_agent_seal"]) {
    assert.match(functionSource(handoff, name), /BrowseMutationGuard::new\(context\)/, name);
  }
  assert.match(read("services/shared_storage/deletion.rs"), /BrowseMutationGuard::new\(context\)/);
  assert.match(read("commands/shared_storage.rs"), /browse::invalidate_context\(context\.inner\(\)\)/);
  assert.match(read("services/shared_storage/shared_storage_manager.rs"), /browse::invalidate_context\(context\)/);
  assert.match(read("services/shared_storage/provider_profiles.rs"), /browse::invalidate_context\(context\)/);
});
