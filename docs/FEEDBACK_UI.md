# Android feedback frontend

## Scope and validation boundary

Repository: `no_land_android`, branch `feat/feedback-program-android`, base `20910cdf9a087492989b03822348088b4801cfb4`.

Frontend ownership is `src/**`, frontend-only scripts/tests, and this document. Existing working-tree changes were preserved and the in-progress UI was completed in place. No files in `src-tauri/**`, `network-agent/**`, or `network-contracts/**` were edited by this frontend task. No dependencies, commits, pushes, deployments, real feedback submissions, or paid provider operations were made.

**No application builds or frontend type-checks were run.** No Cargo, Gradle, Xcode, or Tauri compilation/check/test commands were run. Frontend no-emit type-checking is reserved for the coordinator after all implementers finish; native compilation remains prohibited by the user.

## Changed frontend paths

- `src/features/feedback/`: typed wire/state boundaries, input validation and display selectors, native command facade, program provider, memory summary cache and hook, enrollment/settings modal, card metrics, paginated machine reviews/moderation modal, optional destruction feedback and pure staging flow, accessible dialog and styles.
- `src/app/App.tsx`: provides feedback state independently of the app's operational error/loading path.
- `src/features/dashboard/DashboardScreen.tsx`: top-left large-screen entry and first entry in the actual mobile side menu. Opening the entry is not enrollment.
- `src/features/servers/ServerPickerModal.tsx`: enriches the actual pre-rental cards; keeps provider ordering/filtering and offer identity intact.
- `src/features/settings/SettingsScreen.tsx`: labels the existing reliability preference **Vast reliability**.
- `src/features/shared-storage-manager/InstanceCardActions.tsx`: embeds feedback inside the existing inline destructive confirmation, not a separate paid-action path.
- `src/lib/types.ts`, `src/lib/backend.ts`: additive state and machine identity types, fail-closed feedback state parsing, lossless native machine-ID normalization and destruction invalidation.
- `src/store/appStore.ts`: makes destroy failure observable to the confirmation while retaining the existing native task and generic task-helper behavior.
- `src/locales/{en-US,pt-BR,es,fr,de,it,ja,ko,zh-CN}.json`: feedback copy/variables. New feedback copy has explicit English fallbacks in other locales; human translation/legal review is still required. Existing translations were not replaced.
- `scripts/feedback-ui.test.mjs`: pure Node tests, mocked native seam tests and syntax/source guards; no app build or type-check.
- `scripts/feedback-copy-localization.mjs`: existing frontend-only copy migration helper retained. It was not run with `--write` during completion; it is not required to run the UI/tests.

## Native seam

Every call uses the existing `invokeSafe` wrapper and returns JSON objects, never JSON-encoded strings. There is no feedback HTTP, installation-ID creation, telemetry accumulation, bearer token, recovery code, or browser credential storage in the feature.

| Command | Arguments |
| --- | --- |
| `feedback_program_status` | none; local only |
| `feedback_program_notices` | none; explicit program-modal access/retry only |
| `feedback_program_enroll` | `{ request: { privacyVersion, termsVersion, participationAccepted: true, optionalContextEnabled, optionalProfile } }` |
| `feedback_program_consent` | `{ enabled, optionalContextEnabled }` |
| `feedback_program_delete` | none |
| `feedback_machine_summaries` | `{ machineIds: string[] }`, 1–100 distinct canonical IDs |
| `feedback_machine_reviews` | `{ machineId, cursor: string \| null, limit: 20 }` |
| `feedback_review_report` | `{ reviewId, reason }` |
| `feedback_review_block_author` | `{ reviewId }` |
| `feedback_review_eligibility` | `{ providerInstanceId }`; local frame evidence only |
| `feedback_stage_review` | `{ providerInstanceId, score: number \| null, comment: string \| null }`; durable local checkpoint |

State mirrors the current native `FeedbackProgramState` and `PendingControl` object (`kind`, `requestId`, `expectedEpoch`, `enabled`, `optionalContextEnabled`, `preserveSessions`). Missing, partial, malformed, future-version or unsupported state is disabled. A persisted `enabled: true` bit alone is never sufficient. Raw credentials and deletion status tokens are projected out of native responses; only non-secret references and an optional receipt identifier/status are represented.

Android's current native offer adapter preserves Vast `machine_id` as `machineId: string | null`. The frontend normalizes that verified field to `providerMachineId`; an explicit `providerMachineId` is also accepted. Conflicting fields fail closed. IDs must be positive canonical ASCII decimals within signed PostgreSQL bigint. Numbers, leading zeroes, trailing newlines, host/offer IDs and labels are never used as machine identities. Legacy rental IDs may be converted only when they are positive safe integers; otherwise native must supply the exact `providerInstanceId` string.

`OptionalProfile` now aligns with the native/API allowlist: `country`, `region`, `connectionCategory`, `bandwidthTier`. Fields are optional, trimmed and bounded; blanks are omitted and all fields are discarded when optional sharing is unchecked. No name/email or exact address/network identifiers are requested.

## Enrollment, settings and availability

- Feedback starts fully off. Required participation consent and optional sharing start unchecked; blank optional fields are valid.
- The disclosure explains linked random installation identity, pseudonymity rather than anonymity, required technical context, enrolled-participant visibility, and the **next new rental** rule for enrollment/rejoin.
- The frontend relies on actual native `configured`/`supported`/usable-consent status. Existing platform detection uses Tauri's build target, not user-agent inference. Native reports Keystore, jitter and decoded-frame capability; frontend does not certify those capabilities.
- Status checks on launch, foreground and every 30 seconds are **local-only**. While off, opening program settings (or pressing retry there) is the deliberate exception that invokes native notices fetching. Missing API configuration or native support causes no automatic notices/read request.
- Enrollment requires complete supported notices, HTTPS privacy/terms/deletion links, operator contact, available intake, current applicable policy versions, and explicit checked participation consent. No production domain/contact is invented. Service disclosure and comments render as text.
- Scope expansion requires current notices; reducing sharing, stopping and deleting do not depend on notices loading. Stop remains reachable for an enrolled-but-paused installation so a failed local control checkpoint can be retried.
- Stop/delete immediately hide community data and invalidate caches. Pending withdrawal, deletion and consent synchronization have distinct nonblocking messages. A pending deletion is not presented as completed. Rejoining waits for pending controls to resolve.
- Timed-out control calls are followed by a local status read, not an automatic duplicate control request. Native remains responsible for durable control retries and protected authentication until acknowledgement.

## Cards and viewer data

Machine IDs are deduplicated for the current displayed provider result set and loaded in batches of at most 100. A bounded (500-entry), memory-only **60-second** cache is scoped to installation/viewer, consent epoch, policy versions, optional scope and metric version. Expiry is checked on reads and render. Requests are deduplicated in flight and invalidated generations cannot repopulate another viewer's cache. A failed batch preserves other still-fresh results without extending their original expiry.

Cards replace Vast's displayed percentage only for `no_land`, `ready_and_paired_before_destroy`, retained-history results with at least two eligible rental outcomes and a valid ratio. **Two failures display No Land 0%, not Vast fallback.** Source, session count, original Vast value, outcome explanation and observation dates/as-of are available. Samples of 2–9 outcomes are conservatively marked small; this does not change the threshold or ranking.

Jitter is canonical fully warmed RTT-window data, not RTP jitter or another latency metric. Measured zero is displayed as `0.0 ms`; missing/incompatible observations are N/A. Details include pooled all-routes scope, metric version, min/mean/max, observations/sessions, high/severe shares and compatible route groups. Empty comments display N/A, not a fictitious measured score. Rating-only contributions can still appear in rating summaries.

**Filtering, sorting, recommendation score and automatic rental selection stay Vast-based.** Their labels say Vast reliability. The enrichment never moves/removes an offer, changes its ID, or invokes Rent. The comments icon has a localized accessible name, minimum touch target, and prevents default/stops click propagation. Metrics wrap on small screens.

Caches are cleared for consent/viewer changes, moderation, destruction, foreground and explicit retry. Routine `get_app_state` reads do not continually bust the cache. The seam has no accepted-telemetry/publication invalidation event: remote changes become visible on the next summary expiry or explicit refresh. This is eventual consistency, not a promise of live push updates.

## Reviews and destruction

The reviews modal is keyboard-trapped, Escape-dismissible, scrollable on phones and restores focus to a still-visible trigger. It shows GPU/location, viewer-specific rating average/count, plain-text comments, supplied scores, coarse publication dates and allowlisted historical hardware. It follows opaque cursor pages (`limit: 20`), detects repeated cursors, deduplicates reviews and permits loading successive retained pages. Page state is modal-local, viewer/revision scoped, never persisted; explicit refresh reloads counts and pages. Late responses after identity changes/unmount are ignored. Report/block go through native review-ID commands; acknowledgement refreshes counts/pages without exposing an author ID or altering valid shared setup/jitter history.

The existing destruction confirmation alone requests local native eligibility. Ready/paired flags, connection events, user agent and JavaScript frame counters never authorize a review. A frame-eligible tracked rental receives ten **unselected** 1–10 choices and an optional 2,000-character comment; either field suffices, and whitespace-only comments normalize to null. Copy describes received/submitted frame evidence, not proof of physical scanout or gameplay.

- **Submit and destroy:** await a bounded native local durable draft checkpoint, then invoke the unchanged backup/provider destruction command. No feedback API response is awaited.
- **Skip and destroy:** await the bounded local `feedback_stage_review` call with null/null to clear any retained draft, then continue through existing destruction. This also applies after failed backup/destruction and after reopening the confirmation. Clear failure/timeout warns nonblockingly that earlier feedback may still be submitted; destruction continues. Native checkpoints measurements independently.
- **Cancel:** no staging or destruction from the cancellation action; return to the rental.
- A checkpoint failure/timeout produces a visible nonblocking feedback notice and still runs destruction. A timeout cannot cancel a native checkpoint already underway, so the message says feedback *may* be lost rather than claiming it was not staged.
- Backup/delete failures now reject the store's destroy promise instead of silently returning success. The inline confirmation remains open, retains its known staged draft, and offers a separate retry retaining that draft without another submission, or Skip to durably clear it. Cancellation/reopening or app restart can lose this UI-only saved marker; native must preserve/revision-deduplicate the durable draft. Publication is exclusively native and only after provider-confirmed deletion.
- The existing bulk-close destruction path does not invent or prompt for reviews; native lifecycle/checkpoint behavior still applies.

## Verification and remaining limits

Executed without compilation or external requests:

```sh
node --test scripts/feedback-ui.test.mjs
node scripts/i18n-check.mjs
git --no-pager diff --check -- src scripts/feedback-ui.test.mjs docs/FEEDBACK_UI.md
```

The focused suite has **25 passing source-only tests**: fail-closed state/control objects, lossless canonical IDs, profile consent/allowlist, exact command arguments, viewer/epoch/cache isolation, batching/partial outages/TTL, 0% and fallback behavior, jitter compatibility, public response projection and literal text, local-stage/destroy ordering and errors, unchanged provider filtering/task helpers, locale variables, and TypeScript/TSX **parsing only**. These are not browser interaction tests or TypeScript semantic checks.

Remaining coordinator/release checks:

1. Frontend no-emit type-check only after all implementers finish, per coordination. Native compilation requires future explicit user authorization; none is implied by coordinator follow-up.
2. Device/emulator accessibility/layout and real Keystore/native availability, frame evidence, first/new-rental gating, process death/foreground retry, offline controls, checkpoint failure, backup failure/retry and publication-after-confirmed-delete need runtime smoke tests. Source tests cannot claim those passed.
3. Legal/operator publication, actual configured service URL, moderation ownership, retention/deletion jobs and store disclosures are release configuration/operations, not invented frontend defaults.
4. The consent command cannot acknowledge new privacy/terms versions. An existing installation with mismatched published versions cannot rejoin until native enrollment supports that renewal; the UI fails closed and explains it.
5. The seam exposes receipt status but no recovery-code export or status-token command. The frontend links only the provided external deletion resource and shows native receipt status; it does not expose or invent recovery secrets.
6. Optional profile edits after enrollment have no command in this seam. Settings change consent scope; they do not pretend to PATCH an existing profile. English feedback fallback copy needs human translation and legal review before release.

## Bounded Skip integration follow-up

The integration follow-up also changed native staging/outbox semantics described in `FEEDBACK_NATIVE.md`; the ownership statements above describe the earlier frontend task. Null/null is a local clear, not an empty review submission. Cancel neither stages/clears nor destroys. A successful clear survives restart; clear failure/timeout cannot guarantee suppression of an earlier draft.

Integration validation: 25 Node source tests, 13 Python source-contract tests, localization checks (1,344 keys across nine bundles), Rust formatting only, and `git diff --check`. No compilation or type-check was run in this follow-up.
