import type { FeedbackReviewDraft } from "./types";

export async function destroyWithFeedback({ draft, stage, destroy, persistenceFailed, staged, clear }: {
  draft: FeedbackReviewDraft | null;
  stage: (draft: FeedbackReviewDraft) => Promise<void>;
  destroy: () => Promise<void>;
  persistenceFailed: () => void;
  staged?: () => void;
  clear?: () => Promise<void>;
}) {
  if (draft) {
    try {
      await stage(draft);
      staged?.();
    } catch {
      persistenceFailed();
    }
  }
  if (!draft && clear) {
    try { await clear(); } catch { persistenceFailed(); }
  }
  // Native owns the existing backup/delete path, metric checkpoint, and post-confirmation publication.
  await destroy();
}
