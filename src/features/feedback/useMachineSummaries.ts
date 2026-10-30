import { useEffect, useMemo, useState } from "react";
import { feedbackSummaryCache, invalidateFeedbackData } from "./data";
import { useFeedback } from "./FeedbackProvider";
import { feedbackScope, providerDecimalId } from "./model";
import type { FeedbackMachineSummary } from "./types";

export function useMachineSummaries(open: boolean, offers: ReadonlyArray<{ providerMachineId?: string | null }>) {
  const { status, revision } = useFeedback();
  const scope = feedbackScope(status);
  const idsKey = JSON.stringify([...new Set(offers.flatMap((offer) => {
    const id = providerDecimalId(offer.providerMachineId);
    return id ? [id] : [];
  }))].sort());
  const ids = useMemo<string[]>(() => JSON.parse(idsKey), [idsKey]);
  const [result, setResult] = useState<{ scope: string | null; idsKey: string; summaries: Map<string, FeedbackMachineSummary>; unavailable: boolean }>({ scope: null, idsKey: "", summaries: new Map(), unavailable: false });
  const [expiry, setExpiry] = useState(0);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    if (!open || !scope || ids.length === 0) return;
    feedbackSummaryCache.setScope(scope);
    // Expired values are removed before the next request, not served indefinitely on an API failure.
    const fresh = new Map(ids.flatMap((id) => {
      const summary = feedbackSummaryCache.peek(id);
      return summary ? [[id, summary] as const] : [];
    }));
    setResult({ scope, idsKey, summaries: fresh, unavailable: false });
    void feedbackSummaryCache.read(ids).then((summaries) => {
      if (cancelled) return;
      setResult({ scope, idsKey, summaries, unavailable: false });
      timer = setTimeout(() => setExpiry((value) => value + 1), feedbackSummaryCache.expiresIn(ids));
    }, () => {
      if (cancelled) return;
      // One failed batch must not discard other machines' still-fresh viewer-scoped results.
      const summaries = new Map(ids.flatMap((id) => {
        const summary = feedbackSummaryCache.peek(id);
        return summary ? [[id, summary] as const] : [];
      }));
      setResult({ scope, idsKey, summaries, unavailable: true });
      timer = setTimeout(() => setExpiry((value) => value + 1), feedbackSummaryCache.expiresIn(ids));
    });
    return () => { cancelled = true; if (timer) clearTimeout(timer); };
  }, [open, scope, ids, idsKey, revision, expiry]);

  const current = open && scope !== null && result.scope === scope && result.idsKey === idsKey;
  // Check the cache at render as well, so a delayed timer cannot expose expired/purged data.
  const summaries = new Map(current ? [...result.summaries].filter(([id, summary]) => feedbackSummaryCache.peek(id) === summary) : []);
  return { enabled: scope !== null, summaries, unavailable: current && result.unavailable, retry: invalidateFeedbackData };
}
