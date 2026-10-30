import { translate } from "../../lib/i18n";
import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "../../components/ui/Button";
import { feedbackMachineReviews, feedbackReviewBlockAuthor, feedbackReviewReport } from "./client";
import { invalidateFeedbackData } from "./data";
import { FeedbackDialog } from "./FeedbackDialog";
import { useFeedback } from "./FeedbackProvider";
import { feedbackScope } from "./model";
import type { FeedbackMachineSummary, FeedbackReview } from "./types";

export function MachineReviewsModal({ machineId, gpuName, locationLabel, summary, onClose }: {
  machineId: string;
  gpuName: string;
  locationLabel: string;
  summary?: FeedbackMachineSummary;
  onClose: () => void;
}) {
  const { status, revision } = useFeedback();
  const scope = feedbackScope(status);
  const [page, setPage] = useState<{ scope: string | null; machineId: string; revision: number; reviews: FeedbackReview[]; cursor: string | null; loaded: boolean }>({ scope: null, machineId: "", revision: -1, reviews: [], cursor: null, loaded: false });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [action, setAction] = useState<string | null>(null);
  const [actionMessage, setActionMessage] = useState<string | null>(null);
  const [reporting, setReporting] = useState<string | null>(null);
  const [reason, setReason] = useState("spam");
  const request = useRef(0);
  const actionToken = useRef(0);
  const actionRunning = useRef(false);
  const usedCursors = useRef(new Set<string>());

  const load = useCallback(async (cursor: string | null) => {
    if (!scope) return;
    const version = ++request.current;
    setLoading(true);
    setError(null);
    try {
      const next = await feedbackMachineReviews(machineId, cursor);
      if (version !== request.current) return;
      if (next.nextCursor !== null && (next.nextCursor === cursor || usedCursors.current.has(next.nextCursor))) {
        throw new Error("Repeated pagination cursor");
      }
      if (cursor) usedCursors.current.add(cursor);
      setPage((current) => {
        const previous = cursor && current.scope === scope && current.machineId === machineId && current.revision === revision ? current.reviews : [];
        const seen = new Set(previous.map((review) => review.reviewId));
        return { scope, machineId, revision, reviews: [...previous, ...next.reviews.filter((review) => !seen.has(review.reviewId))], cursor: next.nextCursor, loaded: true };
      });
    } catch {
      if (version === request.current) setError(translate("feedback.6ec723068cc66c58"));
    } finally { if (version === request.current) setLoading(false); }
  }, [machineId, scope, revision]);

  useEffect(() => {
    request.current += 1;
    usedCursors.current.clear();
    setPage({ scope, machineId, revision, reviews: [], cursor: null, loaded: false });
    setReporting(null);
    setError(null);
    setLoading(false);
    if (scope) void load(null);
    return () => { request.current += 1; };
  }, [scope, machineId, revision, load]);

  useEffect(() => {
    actionToken.current += 1;
    actionRunning.current = false;
    setAction(null);
    setActionMessage(null);
    return () => { actionToken.current += 1; };
  }, [scope, machineId]);

  const current = scope !== null && page.scope === scope && page.machineId === machineId && page.revision === revision;
  const reviews = current ? page.reviews : [];

  async function moderate(reviewId: string, kind: "report" | "block") {
    if (!scope || actionRunning.current) return;
    const token = ++actionToken.current;
    actionRunning.current = true;
    setAction(reviewId);
    setActionMessage(null);
    try {
      if (kind === "report") await feedbackReviewReport(reviewId, reason);
      else await feedbackReviewBlockAuthor(reviewId);
      if (token !== actionToken.current) return;
      setReporting(null);
      // Counts and pages share the viewer's visibility rules. Drop every cached count/page on moderation.
      invalidateFeedbackData();
      setActionMessage(translate(kind === "report" ? "feedback.reviews.reported" : "feedback.reviews.blocked"));
    } catch {
      if (token === actionToken.current) setActionMessage(translate("feedback.799dc1d405e72144"));
    } finally {
      if (token === actionToken.current) { actionRunning.current = false; setAction(null); }
    }
  }

  return (
    <FeedbackDialog title={translate("feedback.45eaa437b9e41d66")} onClose={onClose}>
      <p className="font-semibold text-white">{gpuName} · {locationLabel}</p>
      <p>{translate("feedback.28ba2b6df85dac01")} {machineId}{translate("feedback.38d4087b972d69d4")}</p>
      {scope && <p>{translate("feedback.754210ed1286e6c0")} {summary?.reviews.averageScore == null ? translate("feedback.e2f79e5b60330bba") : `${summary.reviews.averageScore.toFixed(1)} / 10`} · {summary?.reviews.ratingCount ?? translate("feedback.e2f79e5b60330bba")} {translate("feedback.61ebf9379940820f")} {summary?.reviews.commentCount ?? translate("feedback.e2f79e5b60330bba")} {translate("feedback.cfc4768ea127d969")}</p>}
      {scope && <Button type="button" variant="ghost" disabled={loading || action !== null} onClick={invalidateFeedbackData} aria-label={translate("feedback.45c0b090192cdca2")}>{translate("generated.0e91610117029a62")}</Button>}
      {!scope && <p role="status">{translate("feedback.63bb781b009600ef")}</p>}
      {actionMessage && <p role="status" className="text-amber-200">{actionMessage}</p>}
      {error && <div role="alert" className="space-y-2"><p>{error}</p><Button type="button" variant="ghost" disabled={loading || !scope} onClick={() => void load(current && page.loaded ? page.cursor : null)}>{translate("feedback.45c0b090192cdca2")}</Button></div>}
      {current && page.loaded && !loading && !error && !reviews.some((review) => review.comment?.trim()) &&
        (summary?.reviews.commentCount === 0 || page.cursor === null) && <p>{translate("feedback.ca657232fa3104a0")}</p>}
      <ul className="space-y-4" aria-label={translate("feedback.8e98ec3ad19d4015")} aria-busy={loading}>
        {reviews.map((review) => (
          <li key={review.reviewId} className="space-y-2 rounded border border-[#6376a1] p-3">
            <p className="font-semibold text-white">{translate("feedback.38e5a46cbc5ad328")} {review.score === null ? translate("feedback.e2f79e5b60330bba") : `${review.score} / 10`} · <time dateTime={review.publishedDate}>{review.publishedDate}</time></p>
            {review.hardwareSnapshot && <p className="text-sm text-[#b4c8de]">{translate("feedback.5cdc3b5107f7dc98")} {[review.hardwareSnapshot.gpuName, review.hardwareSnapshot.rentedGpuCount != null ? translate("feedback.reviews.gpuCount", { count: review.hardwareSnapshot.rentedGpuCount }) : null, review.hardwareSnapshot.country, review.hardwareSnapshot.region].filter(Boolean).join(" · ") || "N/A"}</p>}
            <p className="whitespace-pre-wrap break-words">{review.comment || translate("feedback.reviews.scoreOnly")}</p>
            <div className="flex flex-wrap gap-2">
              <Button type="button" variant="ghost" disabled={action !== null || !scope} onClick={() => { setReporting(reporting === review.reviewId ? null : review.reviewId); setReason("spam"); }} aria-expanded={reporting === review.reviewId}>{translate("feedback.caa4bf2a1441babe")}</Button>
              <Button type="button" variant="ghost" disabled={action !== null || !scope} onClick={() => void moderate(review.reviewId, "block")}>{translate("feedback.117e0030cdf402c6")}</Button>
            </div>
            {reporting === review.reviewId && <div className="space-y-2">
              <label>{translate("feedback.db3a50907695a5a0")}
                <select className="feedback-input" value={reason} onChange={(event) => setReason(event.target.value)}>
                  <option value="spam">{translate("feedback.aa720ff59e6f6799")}</option><option value="abuse">{translate("feedback.d0f2c812503c2e75")}</option><option value="personal_information">{translate("feedback.465f6e73e9bd7ee0")}</option><option value="other">{translate("feedback.7c8c4df258be1f78")}</option>
                </select>
              </label>
              <Button type="button" variant="secondary" disabled={action !== null || !scope} onClick={() => void moderate(review.reviewId, "report")}>{translate("feedback.a44d35311325e654")}</Button>
            </div>}
          </li>
        ))}
      </ul>
      {loading && <p role="status">{translate("feedback.510c765f211f3343")}</p>}
      {current && page.cursor && !error && <Button type="button" variant="secondary" loading={loading} disabled={!scope} onClick={() => void load(page.cursor)}>{translate("feedback.48692a311afa45d4")}</Button>}
      <p className="text-sm text-[#b4c8de]">{translate("feedback.c421a097cde230e2")}</p>
    </FeedbackDialog>
  );
}
