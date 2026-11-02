import { translate } from "../../lib/i18n";
import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import {
  feedbackProgramConsent, feedbackProgramDelete, feedbackProgramEnroll, feedbackProgramStatus,
} from "./client";
import { feedbackSummaryCache, invalidateFeedbackData, subscribeFeedbackInvalidation } from "./data";
import { feedbackScope, unavailableFeedbackStatus } from "./model";
import type { FeedbackEnrollmentRequest, FeedbackProgramState, FeedbackProgramStatus } from "./types";
import "./feedback.css";

interface FeedbackContextValue {
  status: FeedbackProgramStatus;
  busy: boolean;
  revision: number;
  notice: string | null;
  setNotice: (notice: string | null) => void;
  refreshStatus: () => Promise<FeedbackProgramStatus>;
  enroll: (request: FeedbackEnrollmentRequest) => Promise<void>;
  consent: (enabled: boolean, optionalContextEnabled: boolean) => Promise<void>;
  deleteContributions: () => Promise<void>;
}
const FeedbackContext = createContext<FeedbackContextValue | null>(null);

export function FeedbackProvider({ children, persistedState }: { children: ReactNode; persistedState?: FeedbackProgramState }) {
  const [status, setStatus] = useState<FeedbackProgramStatus>(() => unavailableFeedbackStatus("Checking local feedback availability…"));
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  const [notice, setNotice] = useState<string | null>(null);
  const requestVersion = useRef(0);
  const controlRunning = useRef(false);
  const currentStatus = useRef(status);
  const mounted = useRef(true);
  const persistedSignature = JSON.stringify(persistedState ?? null);

  const applyStatus = useCallback((next: FeedbackProgramStatus) => {
    if (!mounted.current) return next;
    feedbackSummaryCache.setScope(feedbackScope(next));
    currentStatus.current = next;
    if (mounted.current) setStatus(next);
    return next;
  }, []);

  const refreshStatus = useCallback(async () => {
    if (controlRunning.current) return currentStatus.current;
    const version = ++requestVersion.current;
    try {
      const next = await feedbackProgramStatus();
      if (version === requestVersion.current) applyStatus(next);
      return next;
    } catch {
      const next = unavailableFeedbackStatus();
      if (version === requestVersion.current) applyStatus(next);
      return next;
    }
  }, [applyStatus]);

  useEffect(() => {
    mounted.current = true;
    const unsubscribe = subscribeFeedbackInvalidation(() => { if (mounted.current) setRevision((value) => value + 1); });
    return () => { mounted.current = false; requestVersion.current += 1; feedbackSummaryCache.setScope(null); unsubscribe(); };
  }, []);

  useEffect(() => {
    if (!controlRunning.current) void refreshStatus();
  }, [persistedSignature, refreshStatus]);

  useEffect(() => {
    const foreground = () => {
      if (document.visibilityState !== "hidden" && !controlRunning.current) {
        invalidateFeedbackData();
        void refreshStatus();
      }
    };
    const interval = setInterval(() => {
      if (document.visibilityState !== "hidden" && !controlRunning.current) void refreshStatus();
    }, 30_000);
    document.addEventListener("visibilitychange", foreground);
    window.addEventListener("focus", foreground);
    return () => { clearInterval(interval); document.removeEventListener("visibilitychange", foreground); window.removeEventListener("focus", foreground); };
  }, [refreshStatus]);

  async function control(action: () => Promise<FeedbackProgramStatus>, disableImmediately: boolean) {
    if (controlRunning.current) throw new Error("A feedback control action is already running.");
    controlRunning.current = true;
    requestVersion.current += 1;
    if (disableImmediately) {
      feedbackSummaryCache.setScope(null);
      currentStatus.current = { ...currentStatus.current, enabled: false };
      setStatus(currentStatus.current);
    }
    invalidateFeedbackData();
    setBusy(true);
    try {
      applyStatus(await action());
    } catch {
      // A timed-out control may still have been persisted by native. Read local truth, never repeat it as a new action.
      const next = await feedbackProgramStatus().catch(() => unavailableFeedbackStatus());
      applyStatus(next);
      throw new Error("Feedback could not confirm this change. Refresh local status before retrying; normal app actions are unaffected.");
    } finally {
      controlRunning.current = false;
      if (mounted.current) setBusy(false);
      invalidateFeedbackData();
    }
  }

  return (
    <FeedbackContext.Provider value={{
      status, busy, revision, notice, setNotice, refreshStatus,
      enroll: (request) => control(() => feedbackProgramEnroll(request), true),
      consent: (enabled, optionalContextEnabled) => control(() => feedbackProgramConsent(enabled, optionalContextEnabled), true),
      deleteContributions: () => control(feedbackProgramDelete, true),
    }}>
      {children}
      {notice && (
        <aside role="status" className="feedback-ui fixed bottom-4 left-4 right-4 z-[120] mx-auto max-w-xl border border-amber-300 bg-[#242035] p-4 text-amber-100 shadow-xl">
          <p>{notice}</p>
          <button type="button" className="feedback-link mt-2" onClick={() => setNotice(null)}>{translate("feedback.fe46408bdfed8578")}</button>
        </aside>
      )}
    </FeedbackContext.Provider>
  );
}

export function useFeedback() {
  const context = useContext(FeedbackContext);
  if (!context) throw new Error("Feedback controls require FeedbackProvider.");
  return context;
}
