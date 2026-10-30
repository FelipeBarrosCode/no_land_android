import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import { registerHooks } from "node:module";
import test from "node:test";
import { fileURLToPath } from "node:url";
import ts from "typescript";

// Node's built-in type stripping executes pure TS; no tsc, emit, bundler, native app, or provider/API requests.
// The native boundary is replaced before importing the real frontend client.
registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === "../../lib/tauri" && context.parentURL?.endsWith("/features/feedback/client.ts")) {
      return { url: "feedback-test:tauri", shortCircuit: true };
    }
    if (specifier.startsWith(".") && context.parentURL?.includes("/features/feedback/") && !/\.[a-z]+$/i.test(specifier)) {
      return nextResolve(new URL(`${specifier}.ts`, context.parentURL).href, context);
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (url === "feedback-test:tauri") return {
      format: "module", shortCircuit: true,
      source: 'export function invokeSafe(command,args) { return globalThis.__feedbackTestInvoke(command,args); } export function isRunningInTauri() { return globalThis.__feedbackTestNative !== false; }',
    };
    return nextLoad(url, context);
  },
});
const model = await import("../src/features/feedback/model.ts");
const { FeedbackSummaryCache } = await import("../src/features/feedback/summaryCache.ts");
const { destroyWithFeedback } = await import("../src/features/feedback/destroyFlow.ts");
const client = await import("../src/features/feedback/client.ts");
const root = fileURLToPath(new URL("../", import.meta.url));
const source = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
const messages = JSON.parse(source("src/locales/en-US.json"));
function assertLocalizedCopy(path, key, copy) {
  assert.ok(source(path).includes(`translate("${key}"`), `${path} uses ${key}`);
  assert.equal(messages[key], copy);
}
const uuid = "123e4567-e89b-42d3-a456-426614174000";
const status = (patch = {}) => ({
  ...model.defaultFeedbackProgram(), enabled: true, enrollmentStatus: "enrolled", installationId: uuid, userId: uuid,
  credentialRef: "protected-reference", deletionCodeRef: "protected-reference", consentEpoch: 1,
  privacyVersion: "1", termsVersion: "1", consentedAt: "2026-10-26T12:00:00Z", configured: true, supported: true,
  statusMessage: null, pendingSessions: 0, ...patch,
});
const summary = (id = "1", eligible = 2, successful = 0) => ({
  providerMachineId: id,
  reliability: { source: "no_land", ratio: eligible >= 2 ? successful / eligible : null, eligibleSessions: eligible,
    successfulSessions: successful, failedSessions: eligible - successful, basis: "ready_and_paired_before_destroy", scope: "retained_history" },
  jitter: null, reviews: { commentCount: 0, ratingCount: 0, averageScore: null }, asOf: "2026-10-26T12:00:00Z",
});
const jitter = (patch = {}) => ({ meanMs: 0, minMs: 0, maxMs: 0, sampleCount: 60, contributingSessions: 1,
  highSampleRatio: 0, severeSampleRatio: 0, metricVersion: model.FEEDBACK_METRIC_VERSION, scope: "all_routes", ...patch });

function findInitializer(text, name, kind = ts.ScriptKind.TSX) {
  const ast = ts.createSourceFile("source.tsx", text, ts.ScriptTarget.Latest, true, kind);
  let result;
  const visit = (node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(ast) === name) result = node.initializer?.getText(ast);
    ts.forEachChild(node, visit);
  };
  visit(ast);
  assert.ok(result, `${name} initializer exists`);
  return result;
}

test("new, partial, malformed and future feedback states fail closed without identity creation", () => {
  for (const value of [undefined, null, {}, { enabled: true }, { ...status(), schemaVersion: 2 },
    status({ consentEpoch: NaN }), status({ optionalContextEnabled: "true" }), status({ installationId: "not-a-uuid" }), status({ userId: `${uuid}\n` }),
    status({ pendingControlAction: "withdrawal" }), status({ enrollmentStatus: "future_status" })]) {
    assert.equal(model.parseFeedbackProgram(value).enabled, false);
  }
  assert.deepEqual(model.parseFeedbackProgram(undefined), model.defaultFeedbackProgram());
  assert.equal(model.parseFeedbackStatus(status()).enabled, true);
  for (const patch of [{ credentialRef: null }, { deletionCodeRef: null }, { termsVersion: null }, { consentEpoch: 0 },
    { configured: false }, { supported: false }, { enabled: false }, { enrollmentStatus: "withdrawn" }]) {
    assert.equal(model.parseFeedbackStatus(status(patch)).enabled, false);
  }
  const clean = model.parseFeedbackStatus({ ...status(), credential: "must-not-be-forwarded", statusToken: "secret" });
  assert.equal("credential" in clean, false);
  assert.equal("statusToken" in clean, false);
});

test("native pending-control objects preserve status but forbid collection and rejoin scope", () => {
  const pendingControlAction = { kind: "withdrawal", requestId: uuid, expectedEpoch: 1, enabled: false, optionalContextEnabled: false, preserveSessions: false };
  const parsed = model.parseFeedbackStatus(status({ pendingControlAction }));
  assert.equal(parsed.enabled, false);
  assert.equal(parsed.pendingControlAction.kind, "withdrawal");
  assert.equal(model.feedbackScope(parsed), null);
});

test("cache scope changes with viewer, consent epoch, policy and optional scope", () => {
  const base = model.feedbackScope(model.parseFeedbackStatus(status()));
  for (const patch of [{ userId: "223e4567-e89b-42d3-a456-426614174000" }, { consentEpoch: 2 }, { privacyVersion: "2" }, { optionalContextEnabled: true }]) {
    assert.notEqual(model.feedbackScope(model.parseFeedbackStatus(status(patch))), base);
  }
  assert.equal(model.feedbackScope(model.parseFeedbackStatus(status({ enabled: false }))), null);
});

test("machine IDs stay exact decimal strings, including beyond Number's safe range", () => {
  for (const id of ["1", "9007199254740993", "9223372036854775807"]) assert.equal(model.providerDecimalId(id), id);
  for (const id of [1, 9007199254740992, "0", "01", "-1", "1.2", "1e3", " 1", "1\n", "1\r\n", "9223372036854775808", "99999999999999999999", null]) {
    assert.equal(model.providerDecimalId(id), null);
  }
  const offer = { id: 42, hostId: 99, machineId: "9007199254740993", score: 80 };
  assert.deepEqual(model.offerMachineIdentity(offer), { ...offer, providerMachineId: "9007199254740993" });
  assert.equal(model.offerMachineIdentity({ id: 42, hostId: 99, hostLabel: "machine 7" }).providerMachineId, null);
  assert.equal(model.offerMachineIdentity({ providerMachineId: "1", machineId: "2" }).providerMachineId, null);
  assert.equal(model.offerMachineIdentity({ providerMachineId: 123, machineId: "123" }).providerMachineId, null);
  assert.equal(model.rentalDecimalId({ instanceId: Number.MAX_SAFE_INTEGER + 1 }), null);
  assert.equal(model.rentalDecimalId({ instanceId: 123 }), "123");
  assert.equal(model.rentalDecimalId({ instanceId: 123, providerInstanceId: "9007199254740993" }), "9007199254740993");
});

test("zero/one eligible sessions retain Vast; two failures display genuine No Land 0%", () => {
  for (const eligible of [0, 1]) assert.deepEqual(model.displayReliability(0.99, summary("1", eligible)), {
    ratio: 0.99, source: "Vast", sessions: eligible, smallSample: false,
  });
  assert.deepEqual(model.displayReliability(0.99, summary()), { ratio: 0, source: "No Land", sessions: 2, smallSample: true });
  assert.equal(model.displayReliability(0.99, summary("1", 2, 1)).ratio, 0.5);
  assert.equal(model.displayReliability(0.99, undefined).source, "Vast");
  assert.equal(model.displayReliability(0.99, { ...summary(), reliability: { ...summary().reliability, ratio: null } }).source, "Vast");
});

test("missing jitter differs from measured zero; incompatible versions/cadences never appear canonical", () => {
  assert.equal(model.canonicalJitter(summary()), null);
  const machine = { ...summary(), jitter: jitter() };
  assert.equal(model.canonicalJitter(machine).meanMs, 0);
  for (const patch of [{ metricVersion: "other" }, { nominalReportIntervalMs: 500 }, { completeness: "partial_window" },
    { thresholdVersion: "other" }, { scope: "route" }]) {
    assert.equal(model.canonicalJitter({ ...machine, jitter: jitter(patch) }), null);
  }
  assert.equal(model.parseMachineSummaries({ summaries: [machine] }, ["1"])[0].jitter.meanMs, 0);
  assert.throws(() => model.parseMachineSummaries({ summaries: [{ ...machine, jitter: jitter({ meanMs: NaN }) }] }, ["1"]));
});

test("summaries reject inconsistent counts, ratios, timestamps and explicit malformed metric metadata", () => {
  const base = summary();
  for (const patch of [
    { asOf: "not-a-date" },
    { reliability: { ...base.reliability, ratio: 0.8 } },
    { reliability: { ...base.reliability, eligibleSessions: 3 } },
    { reliability: { ...base.reliability, eligibleSessions: 1, failedSessions: 1, ratio: 0 } },
    { reviews: { commentCount: 0, ratingCount: 0, averageScore: 8 } },
    { jitter: jitter({ nominalReportIntervalMs: null }) },
    { jitter: jitter({ highSampleRatio: 0.1, severeSampleRatio: 0.2 }) },
  ]) assert.throws(() => model.parseMachineSummaries({ summaries: [{ ...base, ...patch }] }, ["1"]));
  assert.equal(model.displayReliability(0.99, { ...base, reliability: { ...base.reliability, scope: "different_window" } }).source, "Vast");
});

test("optional profile is strictly allowlisted, skippable, and never sent without optional consent", () => {
  assert.deepEqual(model.optionalProfileInput({}, false), {});
  assert.deepEqual(model.optionalProfileInput({ country: "CA", region: "Quebec", name: "not allowed" }, false), {});
  assert.deepEqual(model.optionalProfileInput({ country: " CA ", region: " ", connectionCategory: "fiber", name: "not allowed", email: "not allowed" }, true), { country: "CA", connectionCategory: "fiber" });
  assert.deepEqual(model.optionalProfileInput({ country: "", region: " " }, true), {});
  for (const profile of [{ country: "C" }, { country: "123" }, { region: "a".repeat(81) }, { region: "bad\u0000label" }, { bandwidthTier: "a".repeat(33) }]) {
    assert.equal(model.optionalProfileInput(profile, true), null);
  }
});

test("summaries require every requested identity and project only public fields", () => {
  assert.throws(() => model.parseMachineSummaries({ summaries: [] }, ["1"]));
  assert.throws(() => model.parseMachineSummaries({ summaries: [summary("2")] }, ["1"]));
  assert.throws(() => model.parseMachineSummaries({ summaries: [summary(), summary()] }, ["1"]));
  assert.throws(() => model.parseMachineSummaries({ summaries: [{ ...summary(), providerMachineId: 1 }] }, ["1"]));
  const parsed = model.parseMachineSummaries({ summaries: [{ ...summary(), contributorId: "never-cache-this" }] }, ["1"]);
  assert.equal("contributorId" in parsed[0], false);
});

test("disabled cache never invokes native; enabled requests batch at most 100 unique machines", async () => {
  const batches = [];
  const cache = new FeedbackSummaryCache(async (ids) => { batches.push(ids); return ids.map((id) => summary(id)); });
  const ids = Array.from({ length: 251 }, (_, index) => String(index + 1));
  assert.equal((await cache.read(ids)).size, 0);
  assert.equal(batches.length, 0);
  cache.setScope("viewer:epoch1");
  const result = await cache.read([...ids, ...ids, "0", "invalid"]);
  assert.deepEqual(batches.map((batch) => batch.length), [100, 100, 51]);
  assert.equal(result.size, 251);
  assert.equal(new Set(batches.flat()).size, 251);
  await cache.read(ids);
  assert.equal(batches.length, 3);
});

test("60-second cache expiry is enforced during reads; unavailable data never becomes zero", async () => {
  let now = 0;
  let calls = 0;
  let fail = false;
  const cache = new FeedbackSummaryCache(async (ids) => {
    calls += 1;
    if (fail) throw new Error("offline");
    return ids.map((id) => summary(id));
  }, () => now);
  cache.setScope("viewer1");
  await cache.read(["1"]);
  now = 59_999;
  assert.ok(cache.peek("1"));
  await cache.read(["1"]);
  assert.equal(calls, 1);
  now = 60_000;
  assert.equal(cache.peek("1"), undefined);
  fail = true;
  await assert.rejects(cache.read(["1"]), /offline/);
  assert.equal(cache.peek("1"), undefined);
});

test("one failed summary batch preserves other machines' fresh data and their original expiry", async () => {
  let now = 0;
  const cache = new FeedbackSummaryCache(async (ids) => {
    if (ids.includes("102")) throw new Error("one batch unavailable");
    return ids.map((id) => summary(id));
  }, () => now);
  cache.setScope("viewer");
  await cache.read(["1"]);
  now = 20_000;
  await assert.rejects(cache.read(Array.from({ length: 102 }, (_, index) => String(index + 1))), /one batch unavailable/);
  assert.ok(cache.peek("1"));
  assert.ok(cache.peek("101"));
  assert.equal(cache.peek("102"), undefined);
  assert.equal(cache.expiresIn(["1", "101", "102"]), 40_000);
  now = 60_000;
  assert.equal(cache.peek("1"), undefined);
  assert.ok(cache.peek("101"));
});

test("viewer/consent/moderation invalidation discards in-flight data and cached counts", async () => {
  let release;
  const cache = new FeedbackSummaryCache((ids) => new Promise((resolve) => { release = () => resolve(ids.map((id) => summary(id))); }));
  cache.setScope("viewer1:epoch1");
  const pending = cache.read(["1"]);
  cache.setScope("viewer2:epoch1");
  release();
  assert.equal((await pending).size, 0);
  assert.equal(cache.peek("1"), undefined);
  const next = cache.read(["1"]);
  release();
  await next;
  assert.ok(cache.peek("1"));
  cache.invalidate();
  assert.equal(cache.peek("1"), undefined);
  cache.setScope(null);
  assert.equal((await cache.read(["1"])).size, 0);
});

test("concurrent cards share in-flight summary requests for the same physical machine", async () => {
  let release;
  let calls = 0;
  const cache = new FeedbackSummaryCache((ids) => { calls += 1; return new Promise((resolve) => { release = () => resolve(ids.map((id) => summary(id))); }); });
  cache.setScope("viewer1");
  const one = cache.read(["1", "1"]);
  const two = cache.read(["1"]);
  assert.equal(calls, 1);
  release();
  assert.equal((await one).size, 1);
  assert.equal((await two).size, 1);
});

test("policy links are configured HTTPS notices, not guessed URLs or executable schemes", () => {
  for (const url of ["javascript:alert(1)", "data:text/html,x", "https://user:secret@example.test", "/privacy", "http://example.test"]) assert.equal(model.safePolicyUrl(url), null);
  const document = { schemaVersion: 1, available: true, collectionPaused: false, privacyVersion: "1", termsVersion: "1",
    privacyUrl: "https://example.test/privacy", termsUrl: "https://example.test/terms", deletionUrl: "https://example.test/deletion",
    contact: "Operator support", disclosure: "Test notice", metrics: [] };
  assert.equal(model.parseProgramDocument(document).privacyVersion, "1");
  assert.throws(() => model.parseProgramDocument({ ...document, termsUrl: null }));
  assert.throws(() => model.parseProgramDocument({ ...document, schemaVersion: 2 }));
});

test("reviews paginate with opaque cursors, preserve literal text, and omit private IDs", () => {
  const parsed = model.parseReviewPage({ reviews: [{ reviewId: uuid, score: 10, comment: '<img src=x onerror="attack()">',
    publishedDate: "2026-10-26", contributorId: "private", sessionId: "private", providerInstanceId: "private",
    hardwareSnapshot: { gpuName: "RTX", rentedGpuCount: 1, country: "CA", sshHost: "private" } }], nextCursor: "opaque:page2" });
  assert.equal(parsed.nextCursor, "opaque:page2");
  assert.equal(parsed.reviews[0].comment, '<img src=x onerror="attack()">');
  for (const key of ["contributorId", "sessionId", "providerInstanceId"]) assert.equal(key in parsed.reviews[0], false);
  assert.equal("sshHost" in parsed.reviews[0].hardwareSnapshot, false);
  assert.throws(() => model.parseReviewPage({ reviews: [{ reviewId: uuid, score: 0, comment: null, publishedDate: "2026-10-26" }], nextCursor: null }));
  assert.deepEqual(model.parseReviewPage({ reviews: [], nextCursor: null }), { reviews: [], nextCursor: null });
});

test("review drafts start unselected, normalize blanks, and enforce integer 1–10 / 2000 characters", () => {
  assert.equal(model.reviewDraft(null, "   "), null);
  assert.deepEqual(model.reviewDraft(1, ""), { score: 1, comment: null });
  assert.deepEqual(model.reviewDraft(null, "  useful comment  "), { score: null, comment: "useful comment" });
  for (const score of [0, 11, 1.5, NaN]) assert.equal(model.reviewDraft(score, "comment"), null);
  assert.ok(model.reviewDraft(10, "a".repeat(2000)));
  assert.equal(model.reviewDraft(10, "a".repeat(2001)), null);
  assert.equal(model.reviewDraft(10, "a\u0000b"), null);
});

test("durable stage completes before destroy; skip sends no draft; persistence failure does not block", async () => {
  const events = [];
  const draft = { score: 8, comment: null };
  await destroyWithFeedback({ draft, stage: async () => { events.push("stage-start"); await Promise.resolve(); events.push("durable"); },
    staged: () => events.push("saved"), destroy: async () => { events.push("destroy"); }, persistenceFailed: () => events.push("failure") });
  assert.deepEqual(events, ["stage-start", "durable", "saved", "destroy"]);
  events.length = 0;
  await destroyWithFeedback({ draft: null, stage: async () => { assert.fail("skip must not stage"); }, destroy: async () => { events.push("destroy"); }, persistenceFailed: () => assert.fail() });
  assert.deepEqual(events, ["destroy"]);
  events.length = 0;
  await destroyWithFeedback({ draft, stage: async () => { throw new Error("disk full"); }, destroy: async () => { events.push("destroy"); }, persistenceFailed: () => events.push("warning") });
  assert.deepEqual(events, ["warning", "destroy"]);
  await assert.rejects(destroyWithFeedback({ draft, stage: async () => {}, destroy: async () => { throw new Error("backup failed"); }, persistenceFailed: () => assert.fail() }), /backup failed/);
});

test("real frontend native wrappers send the exact seam arguments, without HTTP or credentials", async () => {
  const calls = [];
  globalThis.__feedbackTestNative = true;
  const document = { schemaVersion: 1, available: true, collectionPaused: false, privacyVersion: "1", termsVersion: "1", privacyUrl: "https://example.test/privacy", termsUrl: "https://example.test/terms", deletionUrl: "https://example.test/deletion", contact: "Support", disclosure: "Notice", metrics: [] };
  globalThis.__feedbackTestInvoke = async (command, args) => {
    calls.push([command, args]);
    if (command === "feedback_program_notices") return document;
    if (command.startsWith("feedback_program_")) return status();
    if (command === "feedback_machine_summaries") return { summaries: args.machineIds.map((id) => summary(id)) };
    if (command === "feedback_machine_reviews") return { reviews: [], nextCursor: null };
    if (command === "feedback_review_eligibility") return { eligible: true };
    return {};
  };
  const enrollment = { privacyVersion: "1", termsVersion: "1", participationAccepted: true, optionalContextEnabled: false, optionalProfile: {} };
  await client.feedbackProgramStatus();
  await client.feedbackProgramNotices();
  await client.feedbackProgramEnroll(enrollment);
  await client.feedbackProgramConsent(false, false);
  await client.feedbackProgramDelete();
  await client.feedbackMachineSummaries(["9007199254740993"]);
  await client.feedbackMachineReviews("9007199254740993", "opaque");
  await client.feedbackReviewReport(uuid, "spam");
  await client.feedbackReviewBlockAuthor(uuid);
  assert.equal(await client.feedbackReviewEligibility("123"), true);
  await client.feedbackStageReview("123", { score: null, comment: "good" });
  await client.feedbackStageReview("123", null);
  assert.deepEqual(calls, [
    ["feedback_program_status", undefined], ["feedback_program_notices", undefined], ["feedback_program_enroll", { request: enrollment }],
    ["feedback_program_consent", { enabled: false, optionalContextEnabled: false }], ["feedback_program_delete", undefined],
    ["feedback_machine_summaries", { machineIds: ["9007199254740993"] }],
    ["feedback_machine_reviews", { machineId: "9007199254740993", cursor: "opaque", limit: 20 }],
    ["feedback_review_report", { reviewId: uuid, reason: "spam" }], ["feedback_review_block_author", { reviewId: uuid }],
    ["feedback_review_eligibility", { providerInstanceId: "123" }], ["feedback_stage_review", { providerInstanceId: "123", score: null, comment: "good" }],
    ["feedback_stage_review", { providerInstanceId: "123", score: null, comment: null }],
  ]);
  const count = calls.length;
  await assert.rejects(client.feedbackMachineSummaries(Array(101).fill("1")));
  await assert.rejects(client.feedbackMachineSummaries([]));
  await assert.rejects(client.feedbackMachineSummaries(["1", "1"]));
  await assert.rejects(client.feedbackMachineSummaries([123]));
  await assert.rejects(client.feedbackMachineReviews("1", "a".repeat(4097)));
  await assert.rejects(client.feedbackStageReview("123", { score: null, comment: "   " }));
  await assert.rejects(client.feedbackStageReview("123", { score: 11, comment: null }));
  assert.equal(await client.feedbackReviewEligibility("not-a-native-rental"), false);
  globalThis.__feedbackTestNative = false;
  await assert.rejects(client.feedbackProgramStatus(), /native app/);
  assert.equal(calls.length, count);
  globalThis.__feedbackTestNative = true;
});

test("picker filtering, ordering and recommendation score stay byte-for-byte provider-based", () => {
  const path = "src/features/servers/ServerPickerModal.tsx";
  const baseline = execFileSync("git", ["--no-pager", "show", `HEAD:${path}`], { cwd: root, encoding: "utf8" });
  assert.equal(findInitializer(source(path), "displayedOffers"), findInitializer(baseline, "displayedOffers"));
  assert.match(source(path), /key=\{offer\.id\}/);
  assert.match(source(path), /setPendingOfferId\(offer\.id\)/);
  assertLocalizedCopy(path, "feedback.7c0d9d97bd35e6ef", "Vast reliability range");
  assertLocalizedCopy(path, "feedback.sort.vastDescending", "Vast reliability: high to low");
  assertLocalizedCopy(path, "feedback.sort.vastAscending", "Vast reliability: low to high");
  assert.match(source(path), /MachineFeedbackMetrics/);
  assert.match(source(path), /MachineReviewsModal/);
});

test("UI source safety and integration guards (not browser interaction tests)", () => {
  const feedbackDir = new URL("../src/features/feedback/", import.meta.url);
  const names = readdirSync(feedbackDir).filter((name) => /\.tsx?$/.test(name));
  const text = names.map((name) => readFileSync(new URL(name, feedbackDir), "utf8")).join("\n");
  assert.doesNotMatch(text, /\bfetch\s*\(|XMLHttpRequest|localStorage|sessionStorage|navigator\.userAgent|dangerouslySetInnerHTML/);
  const program = source("src/features/feedback/FeedbackProgramModal.tsx");
  assert.match(program, /\[accepted, setAccepted\] = useState\(false\)/);
  assert.match(program, /\[optional, setOptional\] = useState\(false\)/);
  assert.match(program, /optionalProfile:|optionalProfile \}/);
  assert.doesNotMatch(program, /type="email"|profile\.name|profile\.email/);
  const destroy = source("src/features/feedback/DestroyFeedbackConfirmation.tsx");
  assert.match(destroy, /useState<number \| null>\(null\)/);
  assert.match(destroy, /feedbackReviewEligibility\(providerInstanceId\)/);
  assert.match(destroy, /eligible && !saved && <fieldset/);
  assertLocalizedCopy("src/features/feedback/DestroyFeedbackConfirmation.tsx", "feedback.8e770652ddca8841", "Submit and destroy");
  assertLocalizedCopy("src/features/feedback/DestroyFeedbackConfirmation.tsx", "feedback.e36ec24fb60dd10d", "Skip and destroy");
  assert.match(destroy, /onClick=\{onCancel\}/);
  assert.match(destroy, /eligibility\.providerInstanceId === providerInstanceId/);
  assert.match(destroy, /maxLength=\{FEEDBACK_COMMENT_LIMIT\}/);
  const metrics = source("src/features/feedback/MachineFeedbackMetrics.tsx");
  assert.match(metrics, /event\.stopPropagation\(\)/);
  assert.match(metrics, /aria-label=\{summary \? translate\("feedback\.reviews\.openCount"/);
  assertLocalizedCopy("src/features/feedback/MachineFeedbackMetrics.tsx", "feedback.reviews.open", "View machine reviews");
  const dialog = source("src/features/feedback/FeedbackDialog.tsx");
  assert.match(dialog, /event\.key === "Tab"/);
  assert.match(dialog, /event\.stopImmediatePropagation\(\)/);
  assert.match(dialog, /previous\.focus\(\)/);
  const dashboard = source("src/features/dashboard/DashboardScreen.tsx");
  assert.equal((dashboard.match(/translate\("feedback\.54698a0574acf1a3"\)/g) ?? []).length, 2);
  assert.match(source("src/app/App.tsx"), /<FeedbackProvider persistedState=\{appState\?\.feedbackProgram\}>/);
});

test("destroy store reports failure to the real confirmation without changing generic task or provider selection semantics", () => {
  const path = "src/store/appStore.ts";
  const baseline = execFileSync("git", ["--no-pager", "show", `HEAD:${path}`], { cwd: root, encoding: "utf8" });
  assert.equal(findInitializer(source(path), "runInstanceTask", ts.ScriptKind.TS), findInitializer(baseline, "runInstanceTask", ts.ScriptKind.TS));
  const completed = findInitializer(source(path), "completed", ts.ScriptKind.TS);
  assert.match(completed, /await destroyInstance\(instanceId\)/);
  assert.match(completed, /return true/);
  assert.match(completed, /false,\s*\)/);
  assert.match(source(path), /if \(!completed\) throw new Error\(translate\("feedback\.538083716511788a"\)\)/);
  assert.doesNotMatch(findInitializer(source("src/lib/backend.ts"), "state"), /invalidateFeedbackData/);
});

test("feedback copy keys exist with matching variables in every supported locale bundle", () => {
  const keys = Object.keys(messages).filter((key) => key.startsWith("feedback."));
  const variables = (message) => [...message.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();
  for (const name of readdirSync(new URL("../src/locales/", import.meta.url)).filter((name) => name.endsWith(".json"))) {
    const bundle = JSON.parse(source(`src/locales/${name}`));
    for (const key of keys) {
      assert.ok(typeof bundle[key] === "string" && bundle[key].trim(), `${name}: ${key}`);
      assert.deepEqual(variables(bundle[key]), variables(messages[key]), `${name}: ${key}`);
    }
  }
});

test("all changed frontend source parses as TypeScript/TSX without building or type-checking", () => {
  const paths = ["src/app/App.tsx", "src/features/dashboard/DashboardScreen.tsx", "src/features/servers/ServerPickerModal.tsx",
    "src/features/settings/SettingsScreen.tsx", "src/features/shared-storage-manager/InstanceCardActions.tsx", "src/lib/backend.ts", "src/lib/types.ts", "src/store/appStore.ts",
    ...readdirSync(new URL("../src/features/feedback/", import.meta.url)).filter((name) => /\.tsx?$/.test(name)).map((name) => `src/features/feedback/${name}`)];
  for (const path of paths) {
    assert.equal(Buffer.from(source(path)).includes(0), false, `${path} contains no literal NUL bytes`);
    const parsed = ts.createSourceFile(path, source(path), ts.ScriptTarget.Latest, true, path.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
    assert.deepEqual(parsed.parseDiagnostics.map((diagnostic) => ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n")), [], path);
  }
});


test("Skip clears a retained draft before destruction and warns without blocking on clear failure", async () => {
  let retained = { score: 8, comment: null };
  const events = [];
  await destroyWithFeedback({ draft: null, stage: async () => assert.fail(),
    clear: async () => { await Promise.resolve(); retained = null; events.push("clear"); },
    destroy: async () => { assert.equal(retained, null); events.push("destroy"); },
    persistenceFailed: () => assert.fail() });
  assert.deepEqual(events, ["clear", "destroy"]);
  events.length = 0;
  await destroyWithFeedback({ draft: null, stage: async () => assert.fail(),
    clear: async () => { throw new Error("disk full"); },
    destroy: async () => { events.push("destroy"); }, persistenceFailed: () => events.push("warning") });
  assert.deepEqual(events, ["warning", "destroy"]);
  const confirmation = readFileSync(new URL("../src/features/feedback/DestroyFeedbackConfirmation.tsx", import.meta.url), "utf8");
  assert.match(confirmation, /feedbackStageReview\(providerInstanceId, null\)/);
  assert.match(confirmation, /onClick=\{onCancel\}/);
});
