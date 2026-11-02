import { translate } from "../../lib/i18n";
import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Button } from "../../components/ui/Button";
import { feedbackProgramNotices } from "./client";
import { FeedbackDialog } from "./FeedbackDialog";
import { useFeedback } from "./FeedbackProvider";
import { optionalProfileInput, safePolicyUrl } from "./model";
import type { FeedbackOptionalProfile, FeedbackProgramDocument } from "./types";

function PolicyLink({ url, children }: { url: string; children: string }) {
  const { setNotice } = useFeedback();
  const safeUrl = safePolicyUrl(url);
  if (!safeUrl) return null;
  return <a className="feedback-link" href={safeUrl} target="_blank" rel="noopener noreferrer" onClick={(event) => {
    event.preventDefault();
    void openUrl(safeUrl).catch(() => setNotice(translate("feedback.f6e2c02158f50a56")));
  }}>{children}</a>;
}

export function FeedbackProgramModal({ onClose }: { onClose: () => void }) {
  const { status, busy, refreshStatus, enroll, consent, deleteContributions } = useFeedback();
  const [program, setProgram] = useState<FeedbackProgramDocument | null>(null);
  const [loadingNotices, setLoadingNotices] = useState(false);
  const [noticesAttempt, setNoticesAttempt] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [accepted, setAccepted] = useState(false);
  const [optional, setOptional] = useState(false);
  const [profile, setProfile] = useState<FeedbackOptionalProfile>({});
  const [scopeChoice, setScopeChoice] = useState(status.optionalContextEnabled);
  const [deleteConfirm, setDeleteConfirm] = useState(false);
  const enrolled = !!status.userId && !["not_enrolled", "deleted"].includes(status.enrollmentStatus);
  const pending = status.pendingControlAction !== null;

  useEffect(() => { setScopeChoice(status.optionalContextEnabled); }, [status.optionalContextEnabled]);

  useEffect(() => {
    let cancelled = false;
    setProgram(null);
    setLoadingNotices(false);
    setAccepted(false);
    setOptional(false);
    setProfile({});
    if (!status.configured || !status.supported) return;
    // Mounting this modal is deliberate access to notices, not enrollment or background enrichment.
    setLoadingNotices(true);
    void feedbackProgramNotices().then((document) => {
      if (!cancelled) { setProgram(document); setError(null); }
    }, () => {
      if (!cancelled) setError(translate("feedback.1087156aea8a8788"));
    }).finally(() => {
      if (!cancelled) {
        setLoadingNotices(false);
        // Notices may explicitly pause native collection; reflect that local truth immediately.
        void refreshStatus();
      }
    });
    return () => { cancelled = true; };
  }, [status.configured, status.supported, noticesAttempt, refreshStatus]);

  async function act(action: () => Promise<unknown>) {
    setError(null);
    try { await action(); setAccepted(false); setDeleteConfirm(false); }
    catch { setError(translate("feedback.e97bdd19cacbcbad")); }
  }

  const noticesReady = !!program?.available && !program.collectionPaused;
  const versionsMatch = !enrolled || (program?.privacyVersion === status.privacyVersion && program?.termsVersion === status.termsVersion);
  const optionalProfile = optionalProfileInput(profile, optional);
  const canJoin = status.configured && status.supported && noticesReady && accepted && optionalProfile !== null &&
    !pending && !busy && versionsMatch && status.enrollmentStatus !== "deletion_pending";
  const canSaveScope = !busy && !pending && scopeChoice !== status.optionalContextEnabled &&
    (!scopeChoice || (noticesReady && versionsMatch));
  const deletionPending = status.enrollmentStatus === "deletion_pending" || status.pendingControlAction?.kind === "deletion";
  const pendingMessage = status.pendingControlAction?.kind === "withdrawal" ? translate("feedback.status.withdrawalPending") :
    status.pendingControlAction?.kind === "deletion" ? translate("feedback.status.deletionPending") : translate("feedback.status.consentPending");

  return (
    <FeedbackDialog title={translate("feedback.54698a0574acf1a3")} onClose={onClose}>
      <section className="space-y-2" aria-label={translate("feedback.224605e2407cf75e")}>
        <h3 className="font-semibold text-white">{translate("feedback.c4ad539bd426dcde")}</h3>
        <p>{translate("feedback.fe2ee1df87bddc6f")}</p>
        <p>{translate("feedback.984ce42c3638842e")}</p>
        <p>{translate("feedback.3e8b68f41c65ac51")}</p>
        <p>{translate("feedback.a59be96446746c12")}</p>
      </section>

      <section className="space-y-2 rounded border border-[#495188] p-3" aria-label={translate("feedback.3059d7fcfe906138")} aria-live="polite">
        <p><strong>{translate("feedback.aa21f1798e0838df")}</strong> {status.enabled ? translate("feedback.6344cde1834f3644") : translate("generated.ca7981b46ecf2c17")}</p>
        <p><strong>{translate("feedback.090fc631289f4af8")}</strong> {status.enrollmentStatus.replace(/_/g, " ")}</p>
        {!status.configured && <p>{translate("feedback.fcfe793aea31c961")}</p>}
        {!status.supported && <p>{translate("feedback.b5dfd08cac1326da")}</p>}
        {status.statusMessage && <p className="whitespace-pre-wrap">{status.statusMessage}</p>}
        {status.jitterCapability && <p><strong>{translate("feedback.617ac6fa3f3f29ac")}</strong> {status.jitterCapability}</p>}
        {status.frameEvidence && <p><strong>{translate("feedback.8cc56a787f4e7db1")}</strong> {status.frameEvidence}</p>}
        {status.deletionStatus && <p><strong>{translate("feedback.0f8fe3e333c1568f")}</strong> {status.deletionStatus}{status.receiptId ? ` · Receipt ${status.receiptId}` : ""}{translate("feedback.447a96f92119aa41")}</p>}
        {pending && <p className="text-amber-200">{pendingMessage}</p>}
        <p>{translate("feedback.d0785fb0652693fc")} {status.pendingSessions}{translate("feedback.1ea84e533fcb5ad1")}</p>
        <Button type="button" variant="ghost" disabled={busy} onClick={() => void act(refreshStatus)}>{translate("feedback.be8927c28ee822e7")}</Button>
      </section>

      {error && <p role="alert" className="text-amber-200">{error}</p>}
      {loadingNotices && <p role="status">{translate("feedback.9cc5aac1cdef1984")}</p>}
      {program ? (
        <section className="space-y-2" aria-label={translate("feedback.473d9f3efa9667aa")}>
          <h3 className="font-semibold text-white">{translate("feedback.cac1c227f4dcff11")}</h3>
          <p className="whitespace-pre-wrap">{program.disclosure}</p>
          <p><strong>{translate("feedback.9b3ec510d8ee0256")}</strong> {program.contact}</p>
          <div className="flex flex-wrap gap-x-5">
            <PolicyLink url={program.privacyUrl}>{translate("feedback.506ff394621596dd")}</PolicyLink>
            <PolicyLink url={program.termsUrl}>{translate("feedback.35dfab3e36191301")}</PolicyLink>
            <PolicyLink url={program.deletionUrl}>{translate("feedback.e65b43841fb8f00e")}</PolicyLink>
          </div>
          <p className="text-sm text-[#adbed6]">{translate("feedback.8b19d285a15ebffd")} {program.privacyVersion} {translate("feedback.5281a8ffbe44d5ab")} {program.termsVersion}</p>
          {!program.available && <p className="text-amber-200">{translate("feedback.ac27f7ceac06fc65")}</p>}
          {program.collectionPaused && <p className="text-amber-200">{translate("feedback.0dab87cd07e2065c")}</p>}
        </section>
      ) : status.configured && status.supported && !loadingNotices ? (
        <Button type="button" variant="ghost" onClick={() => setNoticesAttempt((value) => value + 1)}>{translate("feedback.2ef34c736bac8f6e")}</Button>
      ) : null}

      {!status.enabled && !pending && (
        <section className="space-y-3 border-t border-[#495188] pt-3">
          <label className="feedback-choice items-start">
            <input type="checkbox" checked={accepted} disabled={!noticesReady || busy} onChange={(event) => setAccepted(event.target.checked)} />
            <span>{translate("feedback.1a5e9573e873d96b")}</span>
          </label>
          <label className="feedback-choice items-start">
            <input type="checkbox" checked={optional} disabled={busy} onChange={(event) => {
              setOptional(event.target.checked);
              if (!event.target.checked) setProfile({});
            }} />
            <span>{translate("feedback.2d021560c863b8ee")}</span>
          </label>
          {optional && !enrolled && (
            <fieldset className="grid gap-3 rounded border border-[#495188] p-3 sm:grid-cols-2" disabled={busy}>
              <legend>{translate("feedback.30265808daf51236")}</legend>
              <label>{translate("feedback.2acad5d586137d41")}
                <input className="feedback-input" maxLength={2} autoComplete="off" value={profile.country ?? ""} onChange={(event) => setProfile((current) => ({ ...current, country: event.target.value.toUpperCase().replace(/[^A-Z]/g, "") }))} />
              </label>
              <label>{translate("feedback.857e5c48442e3b74")}
                <input className="feedback-input" maxLength={80} autoComplete="off" value={profile.region ?? ""} onChange={(event) => setProfile((current) => ({ ...current, region: event.target.value }))} />
              </label>
              <label>{translate("feedback.dddb97ab409517b6")}
                <select className="feedback-input" value={profile.connectionCategory ?? ""} onChange={(event) => setProfile((current) => ({ ...current, connectionCategory: event.target.value }))}>
                  <option value="">{translate("feedback.9f755546f6cafcf9")}</option><option value="fiber">{translate("feedback.7ed0a5e33e7be002")}</option><option value="cable">{translate("feedback.eaf9795dc7839efd")}</option><option value="dsl">{translate("feedback.61e06ee14cc6e762")}</option><option value="mobile">{translate("feedback.cbaad3cf4a6507e5")}</option><option value="satellite">{translate("feedback.ad1b033f4885a8bc")}</option><option value="other">{translate("feedback.f97e9da0e3b879f0")}</option>
                </select>
              </label>
              <label>{translate("feedback.89cbd403e4e22c08")}
                <select className="feedback-input" value={profile.bandwidthTier ?? ""} onChange={(event) => setProfile((current) => ({ ...current, bandwidthTier: event.target.value }))}>
                  <option value="">{translate("feedback.9f755546f6cafcf9")}</option><option value="under_25_mbps">{translate("feedback.86cf9d25d9e171d4")}</option><option value="25_100_mbps">{translate("feedback.034f7c7aa932d5a3")}</option><option value="100_500_mbps">{translate("feedback.16a3a706e8c97919")}</option><option value="over_500_mbps">{translate("feedback.1cf4a3d70e4aaa37")}</option>
                </select>
              </label>
              <p className="sm:col-span-2">{translate("feedback.9d7abf0521f80751")}</p>
            </fieldset>
          )}
          {program && !versionsMatch && <p className="text-amber-200">{translate("feedback.74a4b44818d47948")}</p>}
          {optionalProfile === null && <p role="alert" className="text-amber-200">{translate("feedback.enrollment.invalidProfile")}</p>}
          <div className="flex flex-wrap gap-3">
            <Button type="button" disabled={!canJoin} loading={busy} onClick={() => void act(() => {
              if (!program || !canJoin || !optionalProfile) return Promise.resolve();
              if (enrolled) return consent(true, optional);
              return enroll({ privacyVersion: program.privacyVersion, termsVersion: program.termsVersion, participationAccepted: true, optionalContextEnabled: optional, optionalProfile });
            })}>{enrolled ? translate("feedback.2598179d579f9913") : translate("feedback.ed621cd87ba8beef")}</Button>
            <Button type="button" variant="ghost" onClick={onClose}>{translate("feedback.a0e63d7c7125d29a")}</Button>
          </div>
        </section>
      )}

      {enrolled && status.enrollmentStatus === "enrolled" && !pending && (
        <section className="space-y-3 border-t border-[#495188] pt-3">
          {status.enabled && <>
          <label className="feedback-choice items-start">
            <input type="checkbox" checked={scopeChoice} disabled={busy} onChange={(event) => setScopeChoice(event.target.checked)} />
            <span>{translate("feedback.de8fc4524ff17f16")}</span>
          </label>
          <Button type="button" variant="ghost" disabled={!canSaveScope} onClick={() => void act(() => consent(true, scopeChoice))}>{translate("feedback.21017bd3bacd791a")}</Button>
          {scopeChoice && (!noticesReady || !versionsMatch) && <p>{translate("feedback.settings.currentNoticesRequired")}</p>}
          </>}
          <p>{translate("feedback.ea6d0a3574c0b6c3")}</p>
          <Button type="button" variant="secondary" disabled={busy} onClick={() => void act(() => consent(false, false))}>{translate("feedback.32d9a48cbdf0666f")}</Button>
        </section>
      )}

      {enrolled && (
        <section className="space-y-3 border-t border-[#495188] pt-3">
          <p>{translate("feedback.d1e726e4c2748eb5")}</p>
          {!deleteConfirm ? (
            <Button type="button" variant="danger" disabled={busy || deletionPending} onClick={() => setDeleteConfirm(true)}>{translate("feedback.3d821edd098b62af")}</Button>
          ) : (
            <div className="space-y-3 rounded border border-red-400 p-3" role="group" aria-label={translate("feedback.3262081749644ad5")}>
              <p>{translate("feedback.afe13398626a6abd")}</p>
              <div className="flex flex-wrap gap-3">
                <Button type="button" variant="danger" loading={busy} onClick={() => void act(deleteContributions)}>{translate("feedback.67409e8b60e9a6fd")}</Button>
                <Button type="button" variant="ghost" disabled={busy} onClick={() => setDeleteConfirm(false)}>{translate("feedback.d37a8ffcd96dd5c1")}</Button>
              </div>
            </div>
          )}
        </section>
      )}
    </FeedbackDialog>
  );
}
