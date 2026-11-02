# Android native feedback integration

## Scope and validation boundary

Implemented against `feat/feedback-program-android`, base `20910cdf9a087492989b03822348088b4801cfb4` (October 26, 2026). Read both API implementation documents in full before changes. Work is confined to `src-tauri/**` and this document; no changes were needed in `network-agent/**` or `network-contracts/**`. Pre-existing feedback scaffolding was completed, not discarded. The pre-existing deletion of `src-tauri/binaries/ssh-runtime/README.txt` was left alone. Frontend ownership remains separate.

**No native compilation or execution was attempted.** No Cargo check/test/build, Kotlin/C compilation, Gradle, Tauri, Xcode, app/device launch, paid provider operation, real contribution, commit, push, or deployment was run. Source inspection cannot certify Android runtime behavior. The public API URL remains absent unless a distributor explicitly configures it.

Validation performed:

- `rustfmt --edition 2021 --config skip_children=true` on changed Rust implementation files (syntax/formatting only, not type checking).
- `python3 -B src-tauri/android/tests/feedback_source_test.py`: source-contract assertions only; no compiler, emulator, network, or provider client is invoked.
- `git diff --check` on owned native/network paths.
- Rust unit-test **source added, not executed**: strict consent/control parsing, optional profile bounds, large provider IDs and identity separation, state migration, journal round trips, idempotent milestones and scope rebasing, independent review revisions, corruption, retention and bounds, and conservative process-gap behavior. The existing isolated jitter accumulator tests are not evidence of Android metric support.

## Established Android source evidence

| Concern | Actual source / boundary |
| --- | --- |
| Target | `src-tauri/tauri.android.conf.json` identifies `com.noland.connect`; `build.rs::configure_android_cmake` maps Android Rust targets to NDK ABIs, including `aarch64-linux-android` / `arm64-v8a`, and links `mediandk`, `android`, `log`, and OpenSLES. The maintained Android overlay is `src-tauri/android`, not an Apple target inferred from a user agent. |
| Packaging | Existing `scripts/configure-android-project.mjs` copies the maintained overlay into the generated Android project. It was inspected, not edited or executed. Generated project/SDK packaging remains unvalidated. |
| Real runtime gate | `build.rs` emits `noland_feedback_native_android` only on the full Android native-library path. Early `NOLAND_SKIP_APP_BUILD_RS` / `NOLAND_SKIP_NATIVE_BUILD` paths do not enable this capability. Android target, this cfg, working Keystore access, and linked Activity lifecycle callbacks are required before enrollment is usable. |
| Atomic state owner | `AppContext::update_state` still calls the real `StateStore` before committing memory; feedback observes the committed native state afterward. `JsonStateStore` retains existing root-map preservation, including unrelated `moonligConf`, and normal atomic replacement. State schema is **5**, migrated from the branch's original 4. |
| Streaming | Android uses `moonlight/platform/android.rs` and the native `noland_video_renderer_android.c` MediaCodec/Surface renderer. The Kotlin `MediaCodecVideoDecoder.kt` is not assumed to be the authoritative stream path. |
| Frame evidence | Only successful `AMediaCodec_releaseOutputBuffer(..., render=true)` for non-config, non-EOS output increments the new cumulative decoded-output counter. Android atomic accesses prevent cross-thread counter races. The counter resets at native generation start/setup. C stats, normal/static Rust bindings, and `RuntimeStatistics` all carry it. |
| Rental binding | Rust-only `feedback_host_id` is derived from the runtime's actual start request only when native generations match. The feedback adapter resolves that host against the native provisioned-rental record; active-stream identity is also checked by the statistics observer. A connection callback, encoded-input count, or FPS estimate is not the eligibility proof. |
| Pair/setup | Committed `Ready`, setup-complete, actual embedded-pairing and pairing-completed state paths feed idempotent journal milestones, including automatic paths. They do not independently create sessions for pre-existing rentals. |
| Provider deletion | `VastApiClient::destroy_instance` distinguishes positive confirmation (`204`, or `200` with `success: true`) from other successful HTTP responses. The lifecycle service records destruction only for that positive result. Other 2xx results preserve prior operational cleanup but feedback treats them as unknown, not terminal evidence. |
| External/daemon removal | The state-agent can destroy independently, but the Android client does not possess a durable authenticated remote-destroy receipt. A successful owned-instance reconciliation reporting absence only downgrades observation completeness. It does **not** invent a destruction, setup failure, or review. |

`frameEvidence` is `android_mediacodec_decoded_output_submitted_to_surface_not_scanout`. This proves decoded output released to the Android Surface, **not physical display presentation**. The counter extends operational decoder statistics; feedback does not start extra video/network probes while off. Its adapter reads/retains rental evidence only with native consent and a tracked rental.

## Secure storage and reset

### Actual state-secret adapter, not a feedback-only facade

The original Android `AndroidPrivateStateSecrets` in `services/state_secrets.rs` encrypted values using `secure-state-v1/master.key`, a file beside the ciphertext. It is now a real Keystore-backed adapter over shared `services/android_keystore.rs` and `FeedbackNativeBridge.kt`.

- Android Keystore holds non-exportable AES-256/GCM cryptographic keys, **not bearer strings**.
- Ciphertexts use provider-generated random 12-byte IVs, 128-bit GCM tags, and package/account-bound AAD.
- Feedback has a dedicated alias (`com.noland.connect.feedback.aes-gcm.v1`) and encrypted vault at `Context.noBackupFilesDir/noland-feedback-v1/credentials.enc`.
- Existing application state accounts use a separate state-secret key and account-hashed encrypted blobs under `noBackupFilesDir/noland-state-keystore-v1`.
- Writes use Android `AtomicFile`, fsync, directory sync, and authenticated read-back verification. Existing ciphertext with an unavailable/missing key is an error, not an opportunity to regenerate a key over it or use plaintext.
- JNI attaches through the application classloader and uses kept Kotlin entry points. Lifecycle installation verifies the native callback link before reporting support. Missing bridge/key/cipher or failed writes disable feedback; exceptions and secret JSON are not exposed to WebView/logs.
- No plaintext/file-key **write fallback** exists. Old application-secret blobs can be read **only for one-way migration**: obtain the existing old key (never create one), decrypt an old account, persist/verify the Keystore-protected replacement, then remove the legacy blob. The old key is removed after the last `.secret` is migrated. A Keystore read error does not fall back to legacy decryption. Interrupted migration leaves either the verified new account or the old migration material for a later attempt.
- Feedback tokens, enrollment proof, deletion capability and receipt token never use the Moonlight `FileSecretStore`, operational keyring fallbacks, or `state.json`. Existing unrelated Moonlight PEM storage is not reworked by this feature.

The manifest retains `allowBackup=false` / `fullBackupContent=false` and adds API-31+ cloud-backup/device-transfer exclusions in `feedback_data_extraction_rules.xml`. The token blobs also reside in `noBackupFilesDir`. Actual OEM migration/backup behavior still needs device verification; secure data is not portable between devices.

Reset/recovery is fail-closed: missing/malformed/future public state does not recover consent by filling defaults. The protected installation is preserved for explicit withdrawal/deletion/rejoin rather than silently creating an identity. Public/protected identity, policy, epoch and configured API binding must match. Restoring an older enabled state cannot override a protected withdrawal/deletion. A changed compiled API base cannot receive another service's saved credentials. Missing/inaccessible Keystore material is not replaced with plaintext.

The completed-deletion receipt remains protected, and this MVP refuses to automatically recycle a deleted identity. A separate deliberate post-deletion new-identity workflow is not in the coordinated command seam. Likewise, no command exports the deletion recovery code into the WebView; an external-code export experience would require a separately coordinated native capability.

## Public configuration and permissions

`NOLAND_FEEDBACK_API_URL` is a **public compile-time** setting read with `option_env!`; `build.rs` tracks changes. Set it to an operator-controlled HTTPS origin/base or an HTTPS base already ending in `/v1`. Rust appends `/v1` when needed. No production URL/contact is invented. Credentials, URL user-info, query strings and fragments are not accepted as base configuration. Redirects are disabled; the feedback client has 3-second connect and 8-second total timeouts and bounded responses. Failure to construct that constrained client does not fall back to an unconstrained client.

`feedbackProgram` defaults completely off. No identity, key or spool is created by checking local status on a new installation. Disabled status does not fetch `/program` or any feedback API. Deliberate notices/enrollment and previously requested durable account controls are the exceptions.

Permission is checked centrally:

1. Before collection: valid current native consent, full Android adapter/lifecycle availability, matching protected ownership/epoch, tracked new rental, and foreground collection.
2. Again under the journal lock before collection/queue mutation, including consent changes and native state reloads.
3. Before each participant HTTP request and again before returning a response to the WebView; credential material is read from the protected vault at delivery. Withdrawal cancels contribution/read work where possible. An already received server request cannot be retroactively recalled.

API outages do not by themselves withdraw consent or fabricate measurements. Retry/backoff is separate from the normal approximately 60-second delivery cadence. Connectivity/5xx/429 retry with bounded exponential backoff and jitter; Retry-After seconds/dates are respected during the running process. Permanent contribution errors block that record instead of repeatedly retrying it. Authorization failures suspend collection/delivery and persist unusable authorization. Permanent control errors retain the control/authentication for an explicit retry or operator resolution, not endless automatic retries. Backoff timers themselves are process-local; restarting does not preserve a prior Retry-After deadline.

Explicit cached collection pause/policy incompatibility disables collection and is remembered in the vault; it never silently turns participation on again. Current policies/availability are required for an explicit rejoin. The wire contract has no existing-account policy-version migration operation, so changed policy versions fail closed rather than being silently accepted.

## Lifecycle, outbox, and reviews

- Only a new paid-create path can prepare a journal. Enrollment/rejoin never adopts a rental that already exists.
- One UUID is prepared durably before the provider request. The provider label includes that non-secret attempt UUID. The real contract ID is committed as soon as create returns it, **before** the follow-up details fetch can fail. Reconnects/provisioning retries reuse the same rental session.
- A lost create response can be reconciled only against the exact native-generated label in a successful owned-instance listing, never an offer/host/hardware guess. No automatic duplicate paid create is initiated by feedback. If there is an unresolved journaled creation and the provider listing itself fails, another paid creation is deferred until owned rentals can be reconciled. This is a provider-uncertainty guard, not dependence on the feedback API. Provider eventual consistency and real lost-response scenarios need staging/device validation.
- Machine ID comes only from actual `machine_id` on an offer/instance. Missing IDs stay unattributed; verified later instance metadata can fill them, and conflicting bindings block delivery. `host_id`, `hosting_id`, offer ID and rental ID are never substituted for machine ID.
- Existing numeric operational IDs remain compatible. Additive native `providerInstanceId` / `providerOfferId` strings are generated from native integers before JavaScript; rental summaries also expose `providerInstanceId` and `machineId`. UI feedback calls must use these string aliases (or a proven-safe legacy number), never stringify an already rounded Number. Feedback JSON rejects noncanonical, nonpositive or out-of-signed-bigint IDs.
- `feedback-v1/outbox.json` holds the latest cumulative snapshot and independent review draft per rental. Limits: **100 sessions, 5 MiB, 30 days**. Atomic replacement precedes in-memory commit. Completed acknowledged history is evicted before pending lifecycles; evictions are reported locally, not turned into outcomes. Minimal account controls/receipts are outside routine eviction.
- Native events have persisted per-rental sequences and increasing revisions. Ready **and** paired establish absorbing setup success. A first frame separately authorizes the optional review. Repeated events are idempotent. Wall-clock jumps do not replace event ordering.
- Process restart/suspension makes active observation incomplete, preserves prior success/frame evidence, resets metric segment warmup, and never infers failure from a missed upload. State reload revalidates protected consent and does not reuse old operational caches as new feedback sessions.
- Checkpoints occur on native lifecycle pause, foreground/timer (approximately 30 seconds), and before stream stop/teardown. Current frame statistics are also inspected before explicit teardown. Feedback API writes are not awaited by rental/pair/stream/backup/destroy; their errors never become operational command errors.
- The destroy service checkpoints before backup/DELETE. Only positively confirmed provider deletion appends `destroyed` **before** local rental removal. A backup error, failed DELETE, ambiguous 2xx or list failure does not finalize a review/failure. A draft survives operational failure and retries.
- `feedback_stage_review` validates an unselected optional integer 1–10 and/or trimmed plain text (2,000 UTF-16 units), durably stages an independent revision, and performs no HTTP. The sender publishes it only after the destroyed snapshot is acknowledged at the corresponding revision. Null/null durably clears any prior draft and persists an independent increasing review-revision tombstone (including across restart and legacy journal migration). The sender only PUTs a present nonempty draft; it never PUTs the tombstone. Clearing is restricted to active frame-eligible rentals, before any review can be published, so no remote DELETE is needed; a staged review does not itself end the rental.

## Withdrawal, scope changes, deletion

Stop/Delete flips native permission and cancels delivery immediately. **The real atomic state writer saves the control before any wait for an in-flight API/network lock.** Unsent session/review data is cleared; scoped authentication remains protected until acknowledgment. The protected vault mirrors pending intent for reset recovery. An older in-flight acknowledgment cannot overwrite a later stop/delete intent or re-enable local collection.

A withdrawal arriving during an already-issued scope change retains one bounded `previous` consent control. The worker settles/replays that stable request first, binds withdrawal to its acknowledged epoch, and then sends withdrawal, without re-enabling local collection in between. This handles process death/lost consent responses without incrementing the epoch twice. Repeated Stop/Delete reuses pending request identity. Deletion can supersede prior consent because its separate deletion capability is not epoch-bound.

A scope-only change keeps the same rental/journal, strips optional local material immediately, and may keep basic observations local while delivery is paused. Snapshots never contain participant optional profile fields; host hardware/location remains required coarse rental context. The server-accepted epoch is applied once to retained snapshots; retrying an acknowledgment does not keep bumping revisions. A short unobserved control/suspension gap is conservatively incomplete.

Deletion intentionally uses the coordinated recovery operation `POST /v1/deletions {deletionCode, requestId}`, not a blind retry of a revoked participant bearer at `DELETE /me`. Its receipt/status token is durable before participant credentials are removed. Receipt polling uses the distinct status token at `GET /v1/deletions/:receiptId`, only on foreground retries. It is an explicit control continuation, not disabled telemetry. A pending lost enrollment can first be recovered with the original proof to authenticate the requested withdrawal/deletion; installation ID alone never recovers secrets. Expired enrollment recovery remains off and requires ownership/operator resolution.

After acknowledged withdrawal, rejoin creates a fresh acknowledged epoch and empty journal, starting with the **next new rental**. Unsent prior snapshots/reviews are never revived.

## Android jitter: deliberately N/A

The Android operational monitor reaches the existing network agent through `network_monitor/mod.rs` and `reporter.rs`. The agent's `telemetry/metrics.rs` really computes the rolling-60-second RTT-difference EWMA (`abs(delta)` update divided by 16), and reports received counts. However:

- `MetricsSnapshot` does not echo a processed received-sample watermark/batch sequence or a full-window observation proof.
- Client reporter freshness follows the last **sent** batch, which cannot prove that a later heartbeat processed it.
- Operational fail-safe status includes synthetic zero jitter.
- Stream RTT variance/decoded FPS are not this metric, and route windows are not currently contribution-certified.

Therefore `CANONICAL_JITTER_SUPPORTED` is **false**, no reporter calls the contribution accumulator, snapshots have empty `jitterBuckets`, and delivery also rejects nonempty locally recovered buckets on this adapter. `collectorVersion` truthfully identifies Android lifecycle/frame collection without jitter. The standalone gated accumulator is retained as future adapter code, not enabled by observing 60 client reports. No network-agent protocol/source changes or deployment occur merely to claim parity.

Participant reads can still display other clients' canonical community jitter when its metric/cadence/full-window/threshold fields validate. Missing/incompatible readings stay null/N/A, including loss-only/stale windows. A measured valid zero from a compatible remote summary remains zero.

## Tauri command seam

All commands are registered in `lib.rs`, return JSON **objects**, and use the coordinated camelCase arguments. Rejections are ordinary sanitized Tauri errors, not JSON-encoded success strings. Native controls own authentication; the frontend must not call these endpoints directly.

| Command | Arguments | Result |
| --- | --- | --- |
| `feedback_program_status` | none | Local status, no automatic API fetch |
| `feedback_program_notices` | none | Program document from explicit notices access |
| `feedback_program_enroll` | `{request: {privacyVersion, termsVersion, participationAccepted, optionalContextEnabled, optionalProfile}}` | Status |
| `feedback_program_consent` | `{enabled, optionalContextEnabled}` | Status; control can remain pending |
| `feedback_program_delete` | none | Status; control can remain pending |
| `feedback_machine_summaries` | `{machineIds: string[]}` (1–100 distinct after deduplication) | `{summaries: [...]}` with allowlisted fields |
| `feedback_machine_reviews` | `{machineId: string, cursor: string|null, limit: number|null}` | `{reviews: [...], nextCursor}`; default 20, maximum 50 |
| `feedback_review_report` | `{reviewId: string, reason: string}` | `{ok: true}` after acceptance |
| `feedback_review_block_author` | `{reviewId: string}` | `{ok: true}` after acceptance |
| `feedback_review_eligibility` | `{providerInstanceId: string}` | Local-only `{eligible: boolean}` |
| `feedback_stage_review` | `{providerInstanceId: string, score: number|null, comment: string|null}` | `{staged: true}` after durable local save |

Status flattens every plan `feedbackProgram` field and adds `configured`, `supported`, `statusMessage`, `pendingSessions`, `jitterCapability`, `frameEvidence`, and optional `receiptId` / `deletionStatus`. **No credential, proof, deletion code, or receipt capability token is returned.** `enabled` represents usable native consent, not a raw persisted bit.

`pendingControlAction` is null or `{kind, requestId, expectedEpoch, enabled, optionalContextEnabled, preserveSessions, previous?}`. `previous` is the sole additive status-seam extension: a bounded prior consent request during withdrawal. It is never sent as an extra API request-body field. A prepared lost-enrollment withdrawal can temporarily have `expectedEpoch: 0`; native proof recovery binds a positive epoch before the first consent HTTP request. UI clients may ignore the optional prior-control detail while still showing the current pending action.

Other deliberate boundaries, not silent seams: no separate published-review-edit/removal or deletion-code-export commands were added because none were specified; participant account deletion uses the supplied recovery route for safe retry; post-deletion reenrollment and policy-version migration require separate coordinated workflows. No frontend files were edited.

## Required release follow-up (not performed)

1. Only after future explicit user authorization, Android Rust/C/JNI/Kotlin/manifest compilation may be performed; no current authorization is implied. Then check the changed C/Rust statistics ABI together. Source tests do not type-check/link it.
2. Real devices/emulator: Keystore generation, ciphertext tamper/missing-key failure, interrupted atomic writes, old file-key migration, reset/restore and OEM device-transfer exclusion. Hardware/StrongBox backing is not claimed.
3. Stream tests with correct host/generation, actual decoded output, no-frame connection, fast stop, host switch, codec restart, Activity handoff/background and process kill. No physical-presentation evidence is claimed.
4. Staging API fault injection: lost enrollment and consent replies, scope-change-to-withdrawal race, deletion while enrollment/control is in flight, 401/403/429/5xx and Retry-After, permanent validation rejection, receipt recovery/polling, and disabled no-network capture.
5. Fixture/staging lifecycle tests: rejected/ambiguous provider creation, missing/conflicting machine IDs, provisioning/automatic pairing, multiple rentals, ready+paired absorption, failed backup/DELETE and retry, external daemon disappearance, bounded spool/age eviction, and review publication only after positively confirmed deletion. Do not incur paid operations without separate user authorization.
6. Keep canonical Android jitter unsupported until the actual bridge/agent has fresh processed received-sample evidence, a watermark, active-stream and route binding, and a comparable full 60-second window. Tests of a hypothetical accumulator are insufficient.
7. Operator/legal notices, service deployment, moderation, external deletion/export UX, Data safety/privacy disclosures, and retention/deletion jobs remain release responsibilities. No production availability or cross-platform certification is claimed here.

## Bounded Skip integration follow-up

A failed backup/destruction previously retained a draft that a later Skip could publish. The integration follow-up fixes this through the existing coordinated null/null command: atomic outbox replacement clears `review` while persisting the next `reviewRevision`; later drafts advance beyond that tombstone. Older journals default the high-water mark to zero and advance from the maximum existing draft/accepted revision. Session revisions are independent. Clear retains lifecycle/frame evidence and does not finalize the rental. No remote DELETE is necessary because this command rejects ended rentals and the sender cannot publish before confirmed destruction and snapshot acknowledgment. Clear errors remain nonblocking for destruction in the frontend, with an explicit residual-publication warning.

Validated with 25 Node source tests, 13 Python source-contract tests, localization checks, rustfmt only and `git diff --check`. No native compilation, frontend type-check, provider/API call, commit or deployment was performed. Native build/runtime verification requires future explicit user authorization.
