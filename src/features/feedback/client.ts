import { invokeSafe, isRunningInTauri } from "../../lib/tauri";
import {
  parseFeedbackStatus, parseMachineSummaries, parseProgramDocument, parseReviewPage, providerDecimalId, reviewDraft,
} from "./model";
import type { FeedbackEnrollmentRequest, FeedbackReviewDraft } from "./types";

function nativeCall<T>(command: string, args?: Record<string, unknown>, timeoutMs = 12_000): Promise<T> {
  if (!isRunningInTauri()) return Promise.reject(new Error("Feedback requires the native app. Renting and streaming are unaffected."));
  return new Promise<T>((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error("Feedback did not respond in time. Normal app actions remain available.")), timeoutMs);
    invokeSafe<T>(command, args).then(resolve, reject).finally(() => clearTimeout(timeout));
  });
}

export async function feedbackProgramStatus() {
  return parseFeedbackStatus(await nativeCall("feedback_program_status", undefined, 4000));
}
export async function feedbackProgramNotices() {
  return parseProgramDocument(await nativeCall("feedback_program_notices"));
}
export async function feedbackProgramEnroll(request: FeedbackEnrollmentRequest) {
  return parseFeedbackStatus(await nativeCall("feedback_program_enroll", { request }));
}
export async function feedbackProgramConsent(enabled: boolean, optionalContextEnabled: boolean) {
  return parseFeedbackStatus(await nativeCall("feedback_program_consent", { enabled, optionalContextEnabled }));
}
export async function feedbackProgramDelete() {
  return parseFeedbackStatus(await nativeCall("feedback_program_delete"));
}
export async function feedbackMachineSummaries(machineIds: string[]) {
  if (machineIds.length === 0 || machineIds.length > 100 || machineIds.some((id) => !providerDecimalId(id)) || new Set(machineIds).size !== machineIds.length) {
    throw new Error("Feedback needs at most 100 distinct native machine IDs.");
  }
  return parseMachineSummaries(await nativeCall("feedback_machine_summaries", { machineIds }), machineIds);
}
export async function feedbackMachineReviews(machineId: string, cursor: string | null = null) {
  if (!providerDecimalId(machineId)) throw new Error("This offer has no valid native machine ID.");
  if (cursor !== null && (!cursor.trim() || cursor.length > 4096)) throw new Error("Invalid feedback review cursor.");
  return parseReviewPage(await nativeCall("feedback_machine_reviews", { machineId, cursor, limit: 20 }));
}
export async function feedbackReviewReport(reviewId: string, reason: string) {
  await nativeCall("feedback_review_report", { reviewId, reason });
}
export async function feedbackReviewBlockAuthor(reviewId: string) {
  await nativeCall("feedback_review_block_author", { reviewId });
}
export async function feedbackReviewEligibility(providerInstanceId: string): Promise<boolean> {
  if (!providerDecimalId(providerInstanceId)) return false;
  const result = await nativeCall<{ eligible?: unknown }>("feedback_review_eligibility", { providerInstanceId }, 1500);
  return result?.eligible === true;
}
export async function feedbackStageReview(providerInstanceId: string, draft: FeedbackReviewDraft | null) {
  if (!providerDecimalId(providerInstanceId)) throw new Error("Feedback cannot identify this rental safely.");
  const normalized = draft === null ? { score: null, comment: null } : reviewDraft(draft.score, draft.comment ?? "");
  if (!normalized) throw new Error("Provide an optional 1–10 score, a plain-text comment, or both.");
  // This is a bounded local durable checkpoint, not a network submission.
  await nativeCall("feedback_stage_review", { providerInstanceId, score: normalized.score, comment: normalized.comment }, 3000);
}
