import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// No-build policy wiring guards; the shared Rust fixtures exercise the actual
// encrypted inventories and admission races. Never contact or clear live claims.
const read = name => readFileSync(new URL(`../state-agent/crates/noland-storage/${name}`, import.meta.url), "utf8");
const admission = read("src/maintenance.rs");

test("legacy claims default to exclusive admission and absent stable instance scope", () => {
  assert.match(admission, /#\[default\]\s*Exclusive/);
  assert.match(admission, /#\[serde\(default, skip_serializing_if = "RepositoryAccess::is_exclusive"\)\]/);
  assert.match(admission, /#\[serde\(default, skip_serializing_if = "Option::is_none"\)\]/);
  assert.match(admission, /pub instance_id: Option<Uuid>/);
  assert.match(read("src/lib.rs"), /RepositoryAccess, RepositoryActivity/);
  const legacy = admission.split("pub async fn acquire(")[1].split("pub async fn acquire_for_instance(")[0];
  assert.match(legacy, /None,\s*RepositoryAccess::Exclusive/);
});

test("restore readers require both different durable instance and different owner", () => {
  assert.match(admission, /other\.access == RepositoryAccess::RestoreRead/);
  assert.match(admission, /other\.instance_id != activity\.instance_id\s*&& other\.owner != activity\.owner/);
  assert.match(admission, /self\.instance_id\.is_some_and\(\|id\| id\.is_nil\(\)\)/);
  assert.match(admission, /self\.instance_id\.is_none\(\)/);
  assert.match(admission, /RECOVERY_OWNER_PREFIX/);
});

test("all clients observe the same authenticated activity root, with strict inventories", () => {
  assert.match(admission, /format!\("maintenance\/activity\/\{id\}\.enc"\)/);
  assert.match(admission, /list_prefix\(&RemoteKey::new\("maintenance\/activity"\)\)/);
  assert.match(admission, /b"repository-activity-v1"/);
  assert.match(admission, /!observed\.insert\(id\)/);
  assert.match(admission, /activity\.validate\(\)\?/);
});

test("pending deletion fences are checked before and after admission, even for matching reader IDs", () => {
  assert.equal([...admission.matchAll(/crate::pending_deletions\(provider, master\)/g)].length, 2);
  assert.equal([...admission.matchAll(/access != RepositoryAccess::Exclusive \|\| plan\.operation_id != operation_id/g)].length, 2);
});

test("matching restore operation IDs cannot authorize destructive reclamation", () => {
  const execution = read("src/reclamation/execution.rs");
  assert.match(execution, /activities\[0\]\.access != crate::RepositoryAccess::Exclusive/);
  assert.match(execution, /activities\[0\]\.operation_id != plan\.operation_id/);
  assert.match(read("src/reclamation.rs"), /matching_restore_operation_id_never_authorizes_destructive_execution/);
  const recovery = read("src/repository_recovery.rs");
  assert.equal([...recovery.matchAll(/access: Default::default\(\),\s*instance_id: None/g)].length, 5);
});

test("cleanup authenticates the exact execution and does not expire or clear foreign claims by age", () => {
  assert.match(admission, /if current != \*activity/);
  assert.match(admission, /clear_repository_activity\(provider, master, &activity\)/);
  assert.match(admission, /clear_repository_activity\(self\.provider, self\.master, &self\.activity\)/);
  assert.doesNotMatch(admission, /SystemTime|Utc::now|expires_at|lease_timeout|elapsed\(\)/);
});

test("targeted recovery commits to retained claims without expanding receipts to their full inventory", () => {
  const recovery = read("src/repository_recovery.rs");
  const snapshot = recovery.split("pub struct RetainedActivitySnapshot {")[1].split("\n}")[0];
  assert.deepEqual([...snapshot.matchAll(/pub (\w+):/g)].map(match => match[1]), ["count", "hash"]);
  assert.match(recovery, /pub retained_snapshot: Option<RetainedActivitySnapshot>/);
  assert.match(recovery, /sorted\.sort_by_key\(\|activity\| activity\.claim_id\)/);
  assert.match(recovery, /compact_json_hash\(&\(RETAINED_SNAPSHOT_DOMAIN, sorted\)\)/);
  assert.match(recovery, /if snapshot_commitment\(&retained\)\? != receipt\.retained_snapshot/);
  assert.match(recovery, /recovery_receipts_stay_below_16_kib_with_1024_normal_claims/);
});

test("legacy recovery retains nothing implicitly and targeted recovery preserves unrelated ciphertext", () => {
  const recovery = read("src/repository_recovery.rs");
  assert.match(recovery, /recover_repository_activity_preserving\(store, recovery_id, target, Vec::new\(\), authorization\)/);
  assert.match(recovery, /legacy_receipts_decode_without_a_commitment_and_keep_the_exact_old_json_shape/);
  assert.match(recovery, /real_targeted_recovery_preserves_reader_ciphertext_and_backup_data/);
  assert.match(recovery, /changed_or_missing_retained_claim_or_checkpoint_commitment_blocks_resume/);
  assert.match(recovery, /require_no_pending_deletion\(store\)\.await\?/);
});
