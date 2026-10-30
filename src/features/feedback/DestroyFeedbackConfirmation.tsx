import { translate } from "../../lib/i18n";
import { useEffect, useId, useRef, useState } from "react";
import { Button } from "../../components/ui/Button";
import type { RentedInstanceSummary } from "../../lib/types";
import { feedbackReviewEligibility, feedbackStageReview } from "./client";
import { invalidateFeedbackData } from "./data";
import { destroyWithFeedback } from "./destroyFlow";
import { useFeedback } from "./FeedbackProvider";
import { FEEDBACK_COMMENT_LIMIT, feedbackScope, rentalDecimalId, reviewDraft } from "./model";

export function DestroyFeedbackConfirmation({ instance, disabled, onDestroy, onCancel }: {
  instance: RentedInstanceSummary;
  disabled: boolean;
  onDestroy: () => Promise<void>;
  onCancel: () => void;
}) {
  const { status, setNotice } = useFeedback();
  const scope = feedbackScope(status);
  const providerInstanceId = rentalDecimalId(instance);
  const [eligibility, setEligibility] = useState<{ scope: string; providerInstanceId: string; eligible: boolean } | null>(null);
  const [selectedScore, setSelectedScore] = useState<number | null>(null);
  const [comment, setComment] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);
  const id = useId();
  const eligible = scope !== null && eligibility?.scope === scope && eligibility.providerInstanceId === providerInstanceId && eligibility.eligible;
  const draft = reviewDraft(selectedScore, comment);

  useEffect(() => {
    let cancelled = false;
    setEligibility(null);
    setSelectedScore(null);
    setComment("");
    setSaved(false);
    if (!scope || !providerInstanceId) return;
    // Local native evidence, not pairing flags or JS frame counters, authorizes this section.
    void feedbackReviewEligibility(providerInstanceId).then((value) => {
      if (!cancelled) setEligibility({ scope, providerInstanceId, eligible: value });
    }, () => {
      if (!cancelled) setEligibility({ scope, providerInstanceId, eligible: false });
    });
    return () => { cancelled = true; };
  }, [scope, providerInstanceId]);

  async function confirm(submit: boolean) {
    if (inFlight.current || disabled || (submit && (!eligible || !providerInstanceId || !draft))) return;
    inFlight.current = true;
    setSaving(true);
    setError(null);
    try {
      await destroyWithFeedback({
        draft: submit && !saved ? draft : null,
        clear: !submit && scope && providerInstanceId ? async () => {
          await feedbackStageReview(providerInstanceId, null);
          setSaved(false);
        } : undefined,
        stage: (value) => feedbackStageReview(providerInstanceId!, value),
        staged: () => setSaved(true),
        persistenceFailed: () => setNotice(translate("feedback.4a54321041f9fec8")),
        destroy: onDestroy,
      });
      invalidateFeedbackData();
      // If native backup/deletion failed, the rental remains and this confirmation/durable draft remains retryable.
    } catch {
      setError(translate("feedback.538083716511788a"));
    } finally { inFlight.current = false; setSaving(false); }
  }

  return (
    <section className="feedback-ui space-y-3 rounded border border-red-400/60 bg-red-950/25 p-3 text-[#eddaeb]" aria-labelledby={`${id}-title`}>
      <h4 id={`${id}-title`} className="font-semibold text-red-200">{translate("feedback.8d573f8b7e381a39")} {instance.instanceId}?</h4>
      <p>{translate("feedback.2ade73bb98ac1b84")}</p>
      {scope && providerInstanceId && !eligibility && <p role="status">{translate("feedback.9c75f76a870e62ca")}</p>}
      {eligible && !saved && <fieldset className="space-y-3" disabled={disabled || saving}>
        <legend className="font-semibold text-white">{translate("feedback.9cef2ccf592b9212")}</legend>
        <p>{translate("feedback.4400c80039b6982c")}</p>
        <p id={`${id}-scale`}>{translate("feedback.df0dcba4f0dfb062")}</p>
        <div role="radiogroup" aria-label={translate("feedback.3e8132ef12350338")} aria-describedby={`${id}-scale`} className="grid grid-cols-5 gap-2 sm:grid-cols-10">
          {Array.from({ length: 10 }, (_, index) => index + 1).map((value) => <label key={value} className="flex min-h-14 cursor-pointer flex-col items-center justify-center gap-1 rounded border border-[#6376a1] p-2">
            <input type="radio" name={`${id}-score`} value={value} checked={selectedScore === value} onChange={() => setSelectedScore(value)} />
            <span>{value}</span>
          </label>)}
        </div>
        {selectedScore !== null && <button type="button" className="feedback-link" onClick={() => setSelectedScore(null)}>{translate("feedback.f55bb4ae47a3c7d5")}</button>}
        <label className="block" htmlFor={`${id}-comment`}>{translate("feedback.c271966972e67cf9")}
          <textarea id={`${id}-comment`} rows={4} className="feedback-input" maxLength={FEEDBACK_COMMENT_LIMIT} value={comment} onChange={(event) => setComment(event.target.value)} aria-describedby={`${id}-privacy ${id}-count`} />
        </label>
        <p id={`${id}-count`} className="text-sm">{comment.length} / {FEEDBACK_COMMENT_LIMIT} {translate("feedback.25d939ffd0d79b73")}</p>
        <p id={`${id}-privacy`}>{translate("feedback.0906c27744b9e1e1")}</p>
      </fieldset>}
      {saved && <p role="status">{translate("feedback.2bad47162dfd80ce")}</p>}
      {error && <p role="alert" className="text-amber-200">{error}</p>}
      <div className="flex flex-wrap gap-3">
        {eligible && !saved && <Button type="button" variant="danger" disabled={disabled || saving || !draft} loading={saving} onClick={() => void confirm(true)}>{translate("feedback.8e770652ddca8841")}</Button>}
        {saved && <Button type="button" variant="danger" disabled={disabled || saving} onClick={() => void confirm(true)}>{translate("feedback.1f77022d81c8f4a6")}</Button>}
        <Button type="button" variant="danger" disabled={disabled || saving} loading={saving} onClick={() => void confirm(false)}>{eligible || (scope && providerInstanceId) ? translate("feedback.e36ec24fb60dd10d") : translate("feedback.3100ab8bacd41b70")}</Button>
        <Button type="button" variant="ghost" disabled={saving || disabled} onClick={onCancel}>{translate("common.cancel")}</Button>
      </div>
    </section>
  );
}
