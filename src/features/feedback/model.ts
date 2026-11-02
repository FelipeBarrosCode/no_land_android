import type {
  FeedbackEnrollmentStatus,
  FeedbackJitter,
  FeedbackJitterGroup,
  FeedbackMachineSummary,
  FeedbackPendingControl,
  FeedbackOptionalProfile,
  FeedbackProgramDocument,
  FeedbackProgramState,
  FeedbackProgramStatus,
  FeedbackReviewDraft,
  FeedbackReviewPage,
} from "./types";

const MAX_PROVIDER_ID = "9223372036854775807";
export const FEEDBACK_CACHE_TTL_MS = 60_000;
export const FEEDBACK_BATCH_SIZE = 100;
export const FEEDBACK_COMMENT_LIMIT = 2000;
export const FEEDBACK_METRIC_VERSION = "rtt_jitter_ewma_window60_report1s_v1";

export function providerDecimalId(value: unknown): string | null {
  // Native must send the original decimal string, never an already-rounded Number.
  if (typeof value !== "string" || !/^[1-9][0-9]{0,18}(?![\s\S])/.test(value)) return null;
  return value.length < MAX_PROVIDER_ID.length || value <= MAX_PROVIDER_ID ? value : null;
}

export function offerMachineIdentity<T extends { providerMachineId?: unknown; machineId?: unknown }>(offer: T): T & { providerMachineId: string | null } {
  const explicit = providerDecimalId(offer.providerMachineId);
  const native = providerDecimalId(offer.machineId);
  // Android's Rust adapter explicitly preserves Vast machine_id as machineId.
  // A conflicting or malformed explicit identity is not repaired using another field.
  const providerMachineId = offer.providerMachineId !== undefined
    ? explicit && (offer.machineId === undefined || native === explicit) ? explicit : null
    : native;
  return { ...offer, providerMachineId };
}

export function rentalDecimalId(instance: { instanceId: number; providerInstanceId?: string | null }): string | null {
  if (instance.providerInstanceId != null) return providerDecimalId(instance.providerInstanceId);
  // Legacy rental IDs are usable only when JS can represent the exact native integer.
  return Number.isSafeInteger(instance.instanceId) && instance.instanceId > 0
    ? providerDecimalId(String(instance.instanceId))
    : null;
}

export function defaultFeedbackProgram(): FeedbackProgramState {
  return {
    schemaVersion: 1,
    enabled: false,
    enrollmentStatus: "not_enrolled",
    installationId: null,
    userId: null,
    credentialRef: null,
    deletionCodeRef: null,
    consentEpoch: 0,
    privacyVersion: null,
    termsVersion: null,
    consentedAt: null,
    optionalContextEnabled: false,
    pendingControlAction: null,
  };
}

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function text(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}
function nullableText(value: unknown): value is string | null {
  return value === null || text(value);
}
function count(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}
function nonnegative(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}
function ratio(value: unknown): value is number {
  return nonnegative(value) && value <= 1;
}
function score(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 1 && value <= 10;
}

const ENROLLMENT_STATES: readonly FeedbackEnrollmentStatus[] = ["not_enrolled", "enrolling", "enrolled", "withdrawn", "deletion_pending", "deleted", "unavailable"];
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}(?![\s\S])/i;

function pendingControl(value: unknown): FeedbackPendingControl | null | undefined {
  if (value === null) return null;
  if (!record(value) || !["consent", "withdrawal", "deletion"].includes(value.kind as string) ||
      typeof value.requestId !== "string" || !UUID.test(value.requestId) || !count(value.expectedEpoch) ||
      typeof value.enabled !== "boolean" || typeof value.optionalContextEnabled !== "boolean" ||
      !(value.preserveSessions === undefined || typeof value.preserveSessions === "boolean")) return undefined;
  return {
    kind: value.kind as FeedbackPendingControl["kind"], requestId: value.requestId, expectedEpoch: value.expectedEpoch,
    enabled: value.enabled, optionalContextEnabled: value.optionalContextEnabled, preserveSessions: value.preserveSessions === true,
  };
}

export function parseFeedbackProgram(value: unknown): FeedbackProgramState {
  const off = defaultFeedbackProgram();
  if (!record(value) || value.schemaVersion !== 1 || typeof value.enabled !== "boolean" ||
      !ENROLLMENT_STATES.includes(value.enrollmentStatus as FeedbackEnrollmentStatus) || !count(value.consentEpoch) ||
      typeof value.optionalContextEnabled !== "boolean" ||
      ![value.installationId, value.userId, value.credentialRef, value.deletionCodeRef,
        value.privacyVersion, value.termsVersion, value.consentedAt].every(nullableText) ||
      (value.installationId !== null && !UUID.test(value.installationId as string)) ||
      (value.userId !== null && !UUID.test(value.userId as string))) {
    return off;
  }
  const control = pendingControl(value.pendingControlAction);
  if (control === undefined) return off;
  const state: FeedbackProgramState = {
    schemaVersion: 1,
    enabled: value.enabled,
    enrollmentStatus: value.enrollmentStatus as FeedbackEnrollmentStatus,
    installationId: value.installationId as string | null,
    userId: value.userId as string | null,
    credentialRef: value.credentialRef as string | null,
    deletionCodeRef: value.deletionCodeRef as string | null,
    consentEpoch: value.consentEpoch,
    privacyVersion: value.privacyVersion as string | null,
    termsVersion: value.termsVersion as string | null,
    consentedAt: value.consentedAt as string | null,
    optionalContextEnabled: value.optionalContextEnabled,
    pendingControlAction: control,
  };
  state.enabled = state.enabled && state.enrollmentStatus === "enrolled" && state.consentEpoch > 0 &&
    !!state.installationId && !!state.userId && !!state.credentialRef && !!state.deletionCodeRef &&
    !!state.privacyVersion && !!state.termsVersion && !!state.consentedAt &&
    Number.isFinite(Date.parse(state.consentedAt)) && state.pendingControlAction === null;
  return state;
}

export function unavailableFeedbackStatus(message = "Feedback is unavailable. Renting and streaming are unaffected."): FeedbackProgramStatus {
  return { ...defaultFeedbackProgram(), configured: false, supported: false, statusMessage: message, pendingSessions: 0 };
}

export function parseFeedbackStatus(value: unknown): FeedbackProgramStatus {
  if (!record(value) || typeof value.configured !== "boolean" || typeof value.supported !== "boolean" ||
      !nullableText(value.statusMessage) || !count(value.pendingSessions)) return unavailableFeedbackStatus();
  const state = parseFeedbackProgram(value);
  return {
    ...state,
    enabled: state.enabled && value.configured && value.supported,
    configured: value.configured,
    supported: value.supported,
    statusMessage: value.statusMessage,
    pendingSessions: value.pendingSessions,
    jitterCapability: text(value.jitterCapability) ? value.jitterCapability : undefined,
    frameEvidence: text(value.frameEvidence) ? value.frameEvidence : undefined,
    receiptId: text(value.receiptId) ? value.receiptId : null,
    deletionStatus: text(value.deletionStatus) ? value.deletionStatus : null,
  };
}

export function feedbackScope(status: FeedbackProgramStatus): string | null {
  if (!status.enabled || !status.configured || !status.supported || status.pendingControlAction !== null) return null;
  return JSON.stringify([status.schemaVersion, status.installationId, status.userId, status.consentEpoch,
    status.privacyVersion, status.termsVersion, status.optionalContextEnabled, FEEDBACK_METRIC_VERSION]);
}

export function safePolicyUrl(value: unknown): string | null {
  if (!text(value)) return null;
  try {
    const url = new URL(value);
    return url.protocol === "https:" && !url.username && !url.password ? url.href : null;
  } catch { return null; }
}

export function parseProgramDocument(value: unknown): FeedbackProgramDocument {
  if (!record(value) || value.schemaVersion !== 1 || typeof value.available !== "boolean" ||
      typeof value.collectionPaused !== "boolean" || !text(value.privacyVersion) || !text(value.termsVersion) ||
      !text(value.disclosure) || !text(value.contact) || !safePolicyUrl(value.privacyUrl) ||
      !safePolicyUrl(value.termsUrl) || !safePolicyUrl(value.deletionUrl) || !Array.isArray(value.metrics)) {
    throw new Error("The feedback service did not provide complete, supported enrollment notices. Joining is unavailable.");
  }
  return {
    schemaVersion: 1, available: value.available, collectionPaused: value.collectionPaused,
    privacyVersion: value.privacyVersion, termsVersion: value.termsVersion,
    privacyUrl: value.privacyUrl as string, termsUrl: value.termsUrl as string,
    deletionUrl: value.deletionUrl as string, contact: value.contact, disclosure: value.disclosure,
    metrics: value.metrics,
  };
}

function parseJitter(value: unknown): FeedbackJitter | null {
  if (value === null) return null;
  if (!record(value) || !nonnegative(value.meanMs) || !nonnegative(value.minMs) || !nonnegative(value.maxMs) ||
      value.minMs > value.meanMs || value.meanMs > value.maxMs || !count(value.sampleCount) || value.sampleCount === 0 ||
      !count(value.contributingSessions) || value.contributingSessions === 0 || value.contributingSessions > value.sampleCount ||
      !ratio(value.highSampleRatio) || !ratio(value.severeSampleRatio) || value.severeSampleRatio > value.highSampleRatio ||
      !text(value.metricVersion) || !text(value.scope) ||
      (value.nominalReportIntervalMs !== undefined && !nonnegative(value.nominalReportIntervalMs)) ||
      (value.completeness !== undefined && !text(value.completeness)) ||
      (value.thresholdVersion !== undefined && !text(value.thresholdVersion)) ||
      (value.highThresholdMs !== undefined && !nonnegative(value.highThresholdMs)) ||
      (value.severeThresholdMs !== undefined && !nonnegative(value.severeThresholdMs))) {
    throw new Error("The feedback service returned invalid jitter measurements.");
  }
  return {
    meanMs: value.meanMs, minMs: value.minMs, maxMs: value.maxMs, sampleCount: value.sampleCount,
    contributingSessions: value.contributingSessions, highSampleRatio: value.highSampleRatio,
    severeSampleRatio: value.severeSampleRatio, metricVersion: value.metricVersion, scope: value.scope,
    nominalReportIntervalMs: nonnegative(value.nominalReportIntervalMs) ? value.nominalReportIntervalMs : undefined,
    completeness: text(value.completeness) ? value.completeness : undefined,
    thresholdVersion: text(value.thresholdVersion) ? value.thresholdVersion : undefined,
    highThresholdMs: nonnegative(value.highThresholdMs) ? value.highThresholdMs : undefined,
    severeThresholdMs: nonnegative(value.severeThresholdMs) ? value.severeThresholdMs : undefined,
  };
}

export function parseMachineSummaries(value: unknown, requested: readonly string[]): FeedbackMachineSummary[] {
  if (!record(value) || !Array.isArray(value.summaries)) throw new Error("Machine feedback is unavailable.");
  const wanted = new Set(requested);
  const seen = new Set<string>();
  const summaries = value.summaries.map((entry: unknown): FeedbackMachineSummary => {
    if (!record(entry) || !providerDecimalId(entry.providerMachineId) ||
        !wanted.has(entry.providerMachineId as string) || seen.has(entry.providerMachineId as string) ||
        !record(entry.reliability) || !record(entry.reviews) || !text(entry.asOf) || !Number.isFinite(Date.parse(entry.asOf))) {
      throw new Error("The feedback service returned invalid machine summaries.");
    }
    const r = entry.reliability;
    const reviews = entry.reviews;
    if (r.source !== "no_land" || !(r.ratio === null || ratio(r.ratio)) ||
        !count(r.eligibleSessions) || !count(r.successfulSessions) || !count(r.failedSessions) ||
        r.successfulSessions + r.failedSessions !== r.eligibleSessions ||
        !text(r.basis) || !text(r.scope) || !count(reviews.commentCount) || !count(reviews.ratingCount) ||
        !(reviews.averageScore === null || (nonnegative(reviews.averageScore) && reviews.averageScore >= 1 && reviews.averageScore <= 10)) ||
        (r.ratio !== null && (r.eligibleSessions < 2 || Math.abs(r.ratio - r.successfulSessions / r.eligibleSessions) > 1e-9)) ||
        (reviews.ratingCount === 0 && reviews.averageScore !== null) || (reviews.ratingCount > 0 && reviews.averageScore === null)) {
      throw new Error("The feedback service returned invalid machine measurements.");
    }
    const jitter = parseJitter(entry.jitter);
    const jitterGroups: FeedbackJitterGroup[] = [];
    if (Array.isArray(entry.jitterGroups)) {
      for (const group of entry.jitterGroups) {
        if (!record(group) || !["direct", "relay", "mixed"].includes(group.routeMode as string)) continue;
        const parsed = parseJitter(group);
        if (parsed) jitterGroups.push({ ...parsed, routeMode: group.routeMode as FeedbackJitterGroup["routeMode"] });
      }
    }
    seen.add(entry.providerMachineId as string);
    return {
      providerMachineId: entry.providerMachineId as string,
      reliability: {
        source: "no_land", ratio: r.ratio, eligibleSessions: r.eligibleSessions,
        successfulSessions: r.successfulSessions, failedSessions: r.failedSessions, basis: r.basis, scope: r.scope,
        firstObservedAt: text(r.firstObservedAt) ? r.firstObservedAt : null,
        lastObservedAt: text(r.lastObservedAt) ? r.lastObservedAt : null,
      },
      jitter, jitterGroups,
      reviews: { commentCount: reviews.commentCount, ratingCount: reviews.ratingCount, averageScore: reviews.averageScore },
      asOf: entry.asOf,
      firstObservedAt: text(entry.firstObservedAt) ? entry.firstObservedAt : null,
      lastObservedAt: text(entry.lastObservedAt) ? entry.lastObservedAt : null,
    };
  });
  if (seen.size !== wanted.size) throw new Error("The feedback service omitted requested machine summaries.");
  return summaries;
}

export function displayReliability(providerRatio: number, summary: FeedbackMachineSummary | null | undefined) {
  const community = summary?.reliability;
  if (community && community.source === "no_land" && community.eligibleSessions >= 2 &&
      community.basis === "ready_and_paired_before_destroy" && community.scope === "retained_history" &&
      community.ratio !== null && ratio(community.ratio)) {
    return { ratio: community.ratio, source: "No Land", sessions: community.eligibleSessions, smallSample: community.eligibleSessions < 10 };
  }
  return { ratio: providerRatio, source: "Vast", sessions: community?.eligibleSessions ?? 0, smallSample: false };
}

function compatibleJitter(jitter: FeedbackJitter): boolean {
  return jitter.metricVersion === FEEDBACK_METRIC_VERSION &&
    (jitter.nominalReportIntervalMs === undefined || jitter.nominalReportIntervalMs === 1000) &&
    (jitter.completeness === undefined || jitter.completeness === "full_window") &&
    (jitter.thresholdVersion === undefined || jitter.thresholdVersion === "noland_jitter_bands_v1") &&
    (jitter.highThresholdMs === undefined || jitter.highThresholdMs === 7) &&
    (jitter.severeThresholdMs === undefined || jitter.severeThresholdMs === 15);
}

export function canonicalJitter(summary: FeedbackMachineSummary | null | undefined) {
  return summary?.jitter && summary.jitter.scope === "all_routes" && compatibleJitter(summary.jitter) ? summary.jitter : null;
}

export function canonicalJitterGroups(summary: FeedbackMachineSummary | null | undefined) {
  return summary?.jitterGroups?.filter((group) => group.scope === "route" && compatibleJitter(group)) ?? [];
}

export function parseReviewPage(value: unknown): FeedbackReviewPage {
  if (!record(value) || !Array.isArray(value.reviews) || value.reviews.length > 50 ||
      !(value.nextCursor === null || (text(value.nextCursor) && value.nextCursor.length <= 4096))) {
    throw new Error("The feedback service returned an invalid review page.");
  }
  const reviews = value.reviews.map((review: unknown) => {
    if (!record(review) || !text(review.reviewId) || !UUID.test(review.reviewId) || !text(review.publishedDate) ||
        !/^\d{4}-\d{2}-\d{2}(?:$|T)/.test(review.publishedDate) || !Number.isFinite(Date.parse(review.publishedDate)) ||
        !(review.score === null || score(review.score)) ||
        !(review.comment === null || (typeof review.comment === "string" && review.comment.length <= FEEDBACK_COMMENT_LIMIT && !review.comment.includes("\0"))) ||
                (review.score === null && (review.comment === null || !review.comment.trim()))) {
      throw new Error("The feedback service returned an invalid review.");
    }
    const hardware = record(review.hardwareSnapshot) ? review.hardwareSnapshot : null;
    return {
      reviewId: review.reviewId,
      score: review.score,
      comment: review.comment,
      publishedDate: review.publishedDate.slice(0, 10),
      hardwareSnapshot: hardware ? {
        gpuName: typeof hardware.gpuName === "string" ? hardware.gpuName : undefined,
        rentedGpuCount: count(hardware.rentedGpuCount) ? hardware.rentedGpuCount : undefined,
        country: typeof hardware.country === "string" ? hardware.country : undefined,
        region: typeof hardware.region === "string" ? hardware.region : undefined,
      } : null,
    };
  });
  return { reviews, nextCursor: value.nextCursor };
}

export function reviewDraft(selectedScore: number | null, comment: string): FeedbackReviewDraft | null {
  if (selectedScore !== null && !score(selectedScore)) return null;
  const normalized = comment.trim();
  if (comment.length > FEEDBACK_COMMENT_LIMIT || comment.includes("\0") || (selectedScore === null && !normalized)) return null;
  return { score: selectedScore, comment: normalized || null };
}

export function optionalProfileInput(profile: FeedbackOptionalProfile, enabled: boolean): FeedbackOptionalProfile | null {
  if (!enabled) return {};
  const result: FeedbackOptionalProfile = {};
  for (const key of ["country", "region", "connectionCategory", "bandwidthTier"] as const) {
    const value = profile[key]?.trim();
    if (!value) continue;
    const maximum = key === "country" ? 2 : key === "region" ? 80 : 32;
    if (value.length > maximum || /[\u0000-\u001f\u007f]/.test(value) || (key === "country" && !/^[A-Z]{2}$/.test(value))) return null;
    result[key] = value;
  }
  return result;
}
