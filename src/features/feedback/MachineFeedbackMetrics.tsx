import { translate } from "../../lib/i18n";
import { canonicalJitter, canonicalJitterGroups, displayReliability, providerDecimalId } from "./model";
import type { FeedbackMachineSummary } from "./types";

export function MachineFeedbackMetrics({ providerRatio, machineId, enabled, unavailable, summary, onReviews }: {
  providerRatio: number;
  machineId?: string | null;
  enabled: boolean;
  unavailable: boolean;
  summary?: FeedbackMachineSummary;
  onReviews: () => void;
}) {
  const reliability = displayReliability(providerRatio, enabled ? summary : null);
  const jitter = enabled ? canonicalJitter(summary) : null;
  const jitterGroups = enabled ? canonicalJitterGroups(summary) : [];
  const validId = providerDecimalId(machineId);
  const missingReason = translate(!validId ? "feedback.metrics.noMachineId" :
    unavailable ? "feedback.metrics.unavailable" : "feedback.metrics.notObserved");
  return (
    <div className="feedback-ui mt-3 border-t border-[#3e4270] pt-3">
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-2">
        <div className="min-w-0" title={reliability.source === "No Land" ? translate("feedback.f825b16d0727788f") : translate("feedback.1f47b283d453c0ea")}>
          <p className="font-semibold text-[#dff8ff]">{reliability.source === "No Land" ? translate("feedback.metrics.setupReliability") : translate("feedback.35bea2413ebb6e7d")} {Number.isFinite(reliability.ratio) ? `${(reliability.ratio * 100).toFixed(1)}%` : translate("feedback.e2f79e5b60330bba")}</p>
          {enabled && <p className="text-sm text-[#b4c8de]">{reliability.source}{reliability.source === "No Land" ? translate("feedback.metrics.sessionCount", { count: reliability.sessions }) : summary ? translate("feedback.metrics.eligibleCount", { count: reliability.sessions }) : ""}</p>}
          {enabled && reliability.smallSample && <p className="text-sm text-amber-200">{translate("feedback.b951a6a92be4f1e7")}</p>}
        </div>
        {enabled && (
          <>
            <div title={jitter ? translate("feedback.0bb1d620ecd0cece") : missingReason}>
              <p className="font-semibold text-[#dff8ff]">{translate("generated.91fbe4c855fbe075")} {jitter ? `${jitter.meanMs.toFixed(1)} ms` : translate("feedback.e2f79e5b60330bba")}</p>
              {jitter && <p className="text-sm text-[#b4c8de]">{jitter.contributingSessions} {translate("feedback.55be32a742b46ac3")} {jitter.sampleCount} {translate("feedback.bfc5a6b45b6a1612")}</p>}
            </div>
            <button type="button" className="flex min-h-11 items-center gap-2 border border-[#6376a1] px-3 py-2 text-[#bdefff] disabled:opacity-60" disabled={!validId} aria-label={summary ? translate("feedback.reviews.openCount", { count: summary.reviews.commentCount }) : translate("feedback.reviews.open")} title={summary ? translate("feedback.22adae392a3e4e92") : missingReason} onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              onReviews();
            }}>
              <svg aria-hidden="true" viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" strokeWidth="1.7"><path d="M4 4h16v12H9l-5 4V4Z" /><path d="M8 8h8M8 12h5" /></svg>
              <span>{summary && summary.reviews.commentCount > 0 ? summary.reviews.commentCount : translate("feedback.e2f79e5b60330bba")}</span>
            </button>
          </>
        )}
      </div>
      {enabled && (
        <details className="mt-2 text-sm text-[#b4c8de]">
          <summary className="min-h-11 cursor-pointer py-2">{translate("feedback.474b0dfac6e59b7d")}</summary>
          <div className="space-y-1 pb-2">
            <p>{translate("feedback.79b42dcac4d4c662")} {Number.isFinite(providerRatio) ? `${(providerRatio * 100).toFixed(1)}%` : translate("feedback.e2f79e5b60330bba")}{translate("feedback.8f50f51d3cba5f5c")}</p>
            {summary ? <>
              <p>{translate("feedback.2680b53d681c0d37")} {summary.reliability.successfulSessions} {translate("feedback.578c1d3df7db0434")} {summary.reliability.failedSessions} {translate("feedback.f437431e4854c530")} {summary.reliability.eligibleSessions} {translate("feedback.c80f21fdcb7c219f")}</p>
              <p>{translate("feedback.5ab1ad7a5adc58cd")} {summary.reliability.scope}{translate("feedback.53bcd8127815e205")} {summary.asOf}.</p>
              {(summary.reliability.firstObservedAt || summary.firstObservedAt) && <p>{translate("feedback.a4120eab9690cf8b")} {summary.reliability.firstObservedAt || summary.firstObservedAt}</p>}
              {(summary.reliability.lastObservedAt || summary.lastObservedAt) && <p>{translate("feedback.e2a4edccbbc460b2")} {summary.reliability.lastObservedAt || summary.lastObservedAt}</p>}
              {jitter ? <>
                <p>{translate("feedback.2dd902d9657c9cb6")} {jitter.minMs.toFixed(1)} / {jitter.meanMs.toFixed(1)} / {jitter.maxMs.toFixed(1)} {translate("feedback.c439ca26b902578f")} {jitter.scope}{translate("feedback.01b324702bcb8bcb")} {jitter.metricVersion}.</p>
                <p>{translate("feedback.24e8a78abfc68665")} {(jitter.highSampleRatio * 100).toFixed(1)}{translate("feedback.d3fdc84d039d9168")} {(jitter.severeSampleRatio * 100).toFixed(1)}{translate("feedback.2ac296f20c437015")}</p>
              </> : <p>{translate("feedback.bbc3aff0779e70cd")}</p>}
              {jitterGroups.map((group, index) => <p key={`${group.routeMode}-${index}`}>{translate("feedback.bf3ba6591e5d6c2e")} {group.routeMode}: {group.meanMs.toFixed(1)} {translate("feedback.8966f113c3f4f9e2")} {group.minMs.toFixed(1)}–{group.maxMs.toFixed(1)} {translate("feedback.292584090a9847d9")} {group.contributingSessions} {translate("feedback.55be32a742b46ac3")} {group.sampleCount} {translate("feedback.645ceadb07da7fdb")}</p>)}
              <p>{translate("feedback.c5b8de40f2d0a656")} {summary.reviews.averageScore === null ? translate("feedback.e2f79e5b60330bba") : `${summary.reviews.averageScore.toFixed(1)} / 10`} · {summary.reviews.ratingCount} {translate("feedback.61ebf9379940820f")} {summary.reviews.commentCount} {translate("feedback.b2c2c4fbd0c5aa07")}</p>
            </> : <p>{missingReason}</p>}
          </div>
        </details>
      )}
    </div>
  );
}
