"""Source-contract checks only: no Cargo, Kotlin, Gradle, C compiler, device, or HTTP.

Run from any directory: python3 -B src-tauri/android/tests/feedback_source_test.py
These checks deliberately do not claim native compilation or runtime validation.
"""
from pathlib import Path
import re
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[3]
NATIVE = ROOT / "src-tauri"


def source(path):
    return (NATIVE / path).read_text()


def section(text, start, end):
    return text.split(start, 1)[1].split(end, 1)[0]


class FeedbackSourceContract(unittest.TestCase):
    def test_all_coordinated_commands_are_registered_and_return_objects(self):
        names = {
            "feedback_program_status", "feedback_program_notices", "feedback_program_enroll",
            "feedback_program_consent", "feedback_program_delete", "feedback_machine_summaries",
            "feedback_machine_reviews", "feedback_review_report", "feedback_review_block_author",
            "feedback_review_eligibility", "feedback_stage_review",
        }
        commands = source("src/commands/feedback.rs")
        self.assertEqual(set(re.findall(r"pub async fn (feedback_\w+)", commands)), names)
        handler = section(source("src/lib.rs"), ".invoke_handler(", "])")
        for name in names:
            self.assertIn(name, handler)
        self.assertNotIn("Result<String", commands)
        self.assertIn('rename_all = "camelCase"', commands)
        for argument in ["request: EnrollmentRequest", "provider_instance_id: String", "machine_ids: Vec<String>", "cursor: Option<String>", "limit: Option<u32>"]:
            self.assertIn(argument, commands)

    def test_keystore_cryptography_and_real_state_backend_share_adapter(self):
        kotlin = source("android/app/src/main/java/com/noland/connect/feedback/FeedbackNativeBridge.kt")
        for evidence in ["AndroidKeyStore", "KeyGenParameterSpec.Builder", "AES/GCM/NoPadding", "noBackupFilesDir", "cipher.updateAAD", "setRandomizedEncryptionRequired(true)", "stream.fd.sync()", "target.finishWrite", "decrypt(context, target.readFully()"]:
            self.assertIn(evidence, kotlin)
        backend = source("src/services/state_secrets.rs")
        android = section(backend, "impl StateSecretBackend for AndroidPrivateStateSecrets", "pub struct KeychainBackedStateStore")
        self.assertIn('self.protected("stateRead"', android)
        self.assertIn('self.protected("stateWrite"', android)
        self.assertNotIn("master_key()", android)
        put = section(android, "fn put(", "fn delete(")
        self.assertNotIn("fs::write", put)
        self.assertNotIn(".encrypt(", put)
        self.assertIn("self.remove_legacy", put)
        self.assertIn("android_keystore::account", backend)
        self.assertIn("android_keystore::call", source("src/services/feedback/secure.rs"))
        self.assertNotIn("FileSecretStore", source("src/services/feedback/secure.rs"))
        self.assertIn("getClassLoader", source("src/services/android_keystore.rs"))
        self.assertIn("loadClass", source("src/services/android_keystore.rs"))

    def test_backup_and_device_transfer_exclude_app_data(self):
        ns = "{http://schemas.android.com/apk/res/android}"
        app = ET.fromstring(source("android/app/src/main/AndroidManifest.xml")).find("application")
        self.assertEqual(app.attrib[ns + "allowBackup"], "false")
        self.assertEqual(app.attrib[ns + "fullBackupContent"], "false")
        self.assertEqual(app.attrib[ns + "dataExtractionRules"], "@xml/feedback_data_extraction_rules")
        rules = ET.fromstring(source("android/app/src/main/res/xml/feedback_data_extraction_rules.xml"))
        for mode in ["cloud-backup", "device-transfer"]:
            exclusions = {e.attrib["domain"] for e in rules.find(mode).findall("exclude") if e.attrib["path"] == "."}
            self.assertTrue({"root", "file", "database", "sharedpref", "external"} <= exclusions)

    def test_enrollment_proof_is_durable_before_request_and_service_bound(self):
        service = source("src/services/feedback/mod.rs")
        enroll = section(service, "pub async fn enroll(", "pub async fn consent(")
        self.assertLess(enroll.index("secure::write(&vault)"), enroll.index('"/enrollments"'))
        self.assertIn("URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())", enroll)
        self.assertIn("attempt.request != request", enroll)
        self.assertIn("24 * 60 * 60", enroll)
        self.assertIn("vault.api_url != self.api_url", enroll)
        model = section(source("src/models/feedback.rs"), "pub struct FeedbackProgramState", "impl Default for FeedbackProgramState")
        for forbidden in ["pub credential:", "pub recovery_proof:", "pub deletion_code:", "pub status_token:"]:
            self.assertNotIn(forbidden, model)

    def test_provider_machine_identity_has_no_offer_or_host_fallback(self):
        model = source("src/models/vast.rs")
        compact = re.sub(r"\s+", "", model)
        self.assertEqual(compact.count('machine_id:value.get("machine_id").and_then(super::feedback::provider_id)'), 2)
        identifier = section(source("src/models/feedback.rs"), "pub fn provider_id(", "#[cfg(test)]")
        self.assertNotIn("as_f64", identifier)
        self.assertIn("i64::MAX", identifier)
        self.assertIn("provider_instance_id = self.instance.instance_id.map(|id| id.to_string())", source("src/models/app_state.rs"))
        self.assertIn("provider_offer_id: Some(offer.id.to_string())", source("src/services/offer_selector.rs"))
        self.assertIn("provider_instance_id: Some(instance.id.to_string())", source("src/commands/mod.rs"))

    def test_pre_request_journal_and_confirmation_precede_hydration(self):
        orchestration = source("src/services/orchestration.rs")
        self.assertLess(orchestration.index("begin_rental(&context, &offer)"), orchestration.index("let mut instance = match instance_manager"))
        provider = section(source("src/services/vast_api.rs"), "pub async fn create_instance", "pub async fn get_instance")
        self.assertLess(provider.index("on_created(contract_id).await"), provider.index("self.get_instance(contract_id)"))
        self.assertIn("Noland Connect Session {id}", source("src/services/instance_manager.rs"))
        self.assertIn("recover_created", orchestration)
        self.assertIn("if unresolved_creation", orchestration)

    def test_actual_frame_release_counter_is_bridged_with_generation_and_host(self):
        decoder = source("native/noland-moonlight/src/noland_video_renderer_android.c")
        release = decoder.index("AMediaCodec_releaseOutputBuffer")
        count = decoder.index("__atomic_add_fetch(&renderer->decoded_output_frame_count")
        self.assertLess(release, count)
        self.assertIn("if (status != AMEDIA_OK) return", decoder[release:count])
        field = "renderer_decoded_output_frame_count"
        for path in ["native/noland-moonlight/include/noland_moonlight.h", "native/noland-moonlight/src/noland_moonlight.c", "build.rs", "src/moonlight/runtime/mod.rs", "src/services/feedback/mod.rs"]:
            self.assertIn(field, source(path), path)
        runtime = source("src/moonlight/runtime/mod.rs")
        self.assertIn("r.session_generation == stats.session_generation", runtime)
        self.assertIn("feedback_host_id", runtime)
        self.assertIn("embedded_moonlight_host_id", source("src/services/feedback/mod.rs"))
        native = source("native/noland-moonlight/src/noland_moonlight.c")
        self.assertIn("__atomic_load_n(&runtime->renderer.decoded_output_frame_count", native)
        self.assertIn("__atomic_store_n(&runtime->renderer.decoded_output_frame_count, 0U", native)

    def test_destroy_is_independent_and_requires_provider_confirmation(self):
        commands = source("src/commands/mod.rs")
        teardown = section(commands, "async fn teardown_local_instance_session", "pub async fn pause_instance")
        self.assertLess(teardown.index("feedback.checkpoint"), teardown.index("runtime.stop"))
        lifecycle = section(source("src/services/instance_lifecycle.rs"), "pub async fn destroy_instance", "async fn maybe_run_backup_first")
        self.assertLess(lifecycle.index("maybe_run_backup_first"), lifecycle.index("vast.destroy_instance"))
        self.assertLess(lifecycle.index("vast.destroy_instance"), lifecycle.index("Milestone::Destroyed"))
        self.assertLess(lifecycle.index("Milestone::Destroyed"), lifecycle.index("vast.list_instances"))
        self.assertIn("if confirmed", lifecycle)
        service = source("src/services/feedback/mod.rs")
        stage = section(service, "pub async fn stage_review", "pub async fn read_api")
        self.assertIn("self.outbox.save", stage)
        self.assertNotIn(".request(", stage)
        self.assertIn("rental.snapshot.lifecycle.destroyed_at.is_some()", service)
        self.assertNotIn(".destroy_instance(", service)
        self.assertNotIn(".create_instance(", service)

    def test_skip_persists_tombstone_without_empty_put_or_revision_reset(self):
        service = source("src/services/feedback/mod.rs")
        stage = section(service, "pub async fn stage_review", "pub async fn read_api")
        self.assertIn("score.is_none() && comment.is_none()", stage)
        self.assertIn("r.review_revision = revision", stage)
        self.assertIn(".checked_add(1)", stage)
        self.assertIn(".max(r.accepted_review_revision)", stage)
        self.assertLess(stage.index("r.review_revision = revision"), stage.index("self.outbox.save(&next)"))
        self.assertNotIn(".request(", stage)
        outbox = source("src/services/feedback/outbox.rs")
        self.assertIn("#[serde(default)]\n    pub review_revision: u64", outbox)
        self.assertIn("self.snapshot.lifecycle.destroyed_at.is_none()", outbox)
        self.assertIn(".filter(|r| r.revision > rental.accepted_review_revision)", service)

    def test_native_jitter_is_explicitly_unsupported_not_heartbeat_parity(self):
        service = source("src/services/feedback/mod.rs")
        self.assertIn("CANONICAL_JITTER_SUPPORTED: bool = false", service)
        self.assertIn("unsupported: operational agent", service)
        self.assertIn("self.snapshot.jitter_buckets.is_empty()", source("src/services/feedback/outbox.rs"))
        reporter = source("src/network_monitor/reporter.rs")
        self.assertNotIn("observe_jitter", reporter)
        self.assertIn('"jitterMs": 0.0', reporter)  # Existing operational fallback is never feedback.

    def test_queue_bounds_recovery_and_durable_controls(self):
        outbox = source("src/services/feedback/outbox.rs")
        for evidence in ["MAX_SESSIONS: usize = 100", "MAX_BYTES: usize = 5 * 1024 * 1024", "30 * 24 * 60 * 60", "write_atomically", "rental.gap()", "self.snapshot.lifecycle.observation_complete = false"]:
            self.assertIn(evidence, outbox)
        service = source("src/services/feedback/mod.rs")
        for evidence in ["owns_journal", "self.outbox.save(&next)", "pending_control = Some(control.clone())", "vault.pending_control.clone()", "self.desired_disabled", "cancelled.as_mut().enable()", "Policy::none()", "self.poll_deletion().await"]:
            self.assertIn(evidence, service)
        deletion = section(service, "if c.kind == ControlKind::Deletion {", "let ack: ConsentAck")
        self.assertLess(deletion.index("vault.receipt = Some"), deletion.index("vault.participant = None"))
        self.assertIn('"/deletions"', deletion)
        self.assertIn("receipt.status_token", service)

    def test_stop_intent_is_durable_without_waiting_for_an_inflight_api(self):
        service = source("src/services/feedback/mod.rs")
        stop = section(service, "async fn queue_stop(", "async fn persist_control(")
        self.assertNotIn("self.controls.lock", stop)
        self.assertNotIn("self.network.lock", stop)
        self.assertNotIn(".request(", stop)
        self.assertLess(stop.index("self.desired_disabled.store(true"), stop.index("context.update_state"))
        self.assertLess(stop.index("context.update_state"), stop.index("self.persist_control"))
        self.assertIn("previous", stop)
        self.assertIn("next.expected_epoch = ack.consent_epoch", service)
        self.assertIn("next.previous = None", service)
        self.assertIn("vault.pending_control=context.load_state().await.feedback_program.pending_control_action;", re.sub(r"\s+", "", service))

    def test_real_android_runtime_gate_and_public_compile_time_configuration(self):
        build = source("build.rs")
        self.assertIn("rustc-check-cfg=cfg(noland_feedback_native_android)", build)
        self.assertLess(build.index("NOLAND_SKIP_NATIVE_BUILD"), build.index("rustc-cfg=noland_feedback_native_android"))
        self.assertIn('option_env!("NOLAND_FEEDBACK_API_URL")', source("src/services/feedback/mod.rs"))
        self.assertIn('url.scheme() != "https"', source("src/services/feedback/mod.rs"))
        self.assertIn("noland_feedback_native_android", source("src/services/feedback/mod.rs"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
