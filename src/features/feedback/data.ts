import { feedbackMachineSummaries } from "./client";
import { FeedbackSummaryCache } from "./summaryCache";

export const feedbackSummaryCache = new FeedbackSummaryCache(feedbackMachineSummaries);
const listeners = new Set<() => void>();

export function invalidateFeedbackData() {
  feedbackSummaryCache.invalidate();
  listeners.forEach((listener) => listener());
}

export function subscribeFeedbackInvalidation(listener: () => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
