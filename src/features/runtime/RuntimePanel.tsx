import { useEffect, useRef, useState } from "react";
import { Button } from "../../components/ui/Button";
import { getInstanceRuntimeStatus, instanceRuntimeAction } from "../../lib/backend";
import { useLocalization } from "../../lib/i18n";
import type { RuntimeAction, RuntimePlan, RuntimeStatus } from "../../lib/types";
import { createRuntimePoller, isRemoteExecutablePath, isRuntimeInstalling, parseRuntimePlan, runtimeError } from "./runtimeModel";

export function RuntimePanel({ instanceId, onLibraryChanged }: { instanceId: number; onLibraryChanged: () => void }) {
  const { t } = useLocalization();
  const [expanded, setExpanded] = useState(false);
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [plan, setPlan] = useState<RuntimePlan | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [loadError, setLoadError] = useState("");
  const [path, setPath] = useState("");
  const [diagnostics, setDiagnostics] = useState<unknown>(null);
  const epoch = useRef(0);
  const operation = useRef(false);
  const libraryChanged = useRef(onLibraryChanged);
  libraryChanged.current = onLibraryChanged;
  const previousRuntime = useRef<string | null>(null);

  useEffect(() => { ++epoch.current; return () => { ++epoch.current; }; }, [instanceId]);

  useEffect(() => {
    if (!expanded) return;
    const poller = createRuntimePoller({
      fetch: () => getInstanceRuntimeStatus(instanceId),
      shouldPoll: () => document.visibilityState === "visible" && !operation.current,
      onValue: (next) => { setStatus(next); setLoadError(""); },
      onError: (e) => setLoadError(runtimeError(e)),
    });
    const onVisibility = () => {
      // Require a fresh measured plan after returning from another app.
      setPlan(null);
      if (document.visibilityState === "visible") void poller.tick();
    };
    void poller.tick();
    const timer = window.setInterval(() => void poller.tick(), 3000);
    document.addEventListener("visibilitychange", onVisibility);
    return () => { poller.stop(); window.clearInterval(timer); document.removeEventListener("visibilitychange", onVisibility); };
  }, [instanceId, expanded]);

  useEffect(() => {
    const id = status?.details?.runtimeId ?? null;
    if (id && previousRuntime.current !== id) libraryChanged.current();
    previousRuntime.current = id;
  }, [status?.details?.runtimeId]);

  async function action(name: RuntimeAction, appId?: string) {
    if (operation.current || (name === "install" && !plan)) return;
    operation.current = true;
    const current = epoch.current;
    setBusy(true); setError("");
    try {
      const result = await instanceRuntimeAction(instanceId, name, name === "register" ? path : undefined, appId, name === "install" ? plan?.manifest.sha256 : undefined);
      if (epoch.current !== current) return;
      if (name === "plan") {
        const reviewed = parseRuntimePlan(result);
        if (!reviewed) throw new Error(t("runtime.invalidPlan"));
        setPlan(reviewed);
      }
      if (name === "doctor") setDiagnostics(result);
      if (name === "install") setPlan(null);
      const next = await getInstanceRuntimeStatus(instanceId);
      if (epoch.current !== current) return;
      setStatus(next);
      if (["enable", "register", "rollback"].includes(name)) libraryChanged.current();
    } catch (e) { if (epoch.current === current) setError(runtimeError(e)); }
    finally { operation.current = false; if (epoch.current === current) setBusy(false); }
  }

  const phase = status?.details?.progress?.phase;
  const installing = isRuntimeInstalling(phase);
  const touch = "min-h-11 w-full sm:w-auto";
  const mib = (n: number) => (n / 1048576).toFixed(1);
  return (
    <details className="mb-4 min-w-0 border border-[#283252] bg-[#0d132b] p-4 text-sm text-[#a8bed6]" onToggle={(event) => setExpanded(event.currentTarget.open)}>
      <summary className="min-h-11 cursor-pointer py-3 font-display text-white">{t("runtime.title")}</summary>
      <p className="my-3">{t("runtime.description")}</p>
      <p className="my-3 text-neon-cyan">{t("runtime.location")}</p>
      <p>{t("runtime.warning")}</p>
      <p className="my-3 break-words" role="status">{status ? status.enabled ? `${status.details?.runtimeId ?? t("runtime.absent")} · ${phase ?? "absent"}` : t("runtime.disabled") : t("runtime.loading")}</p>
      {status && !status.releaseConfigured && <p className="my-2 text-amber-300">{t("runtime.unconfigured")}</p>}
      {(error || loadError) && <p role="alert" className="my-2 break-words text-red-300">{error || loadError}</p>}
      {status?.details?.progress?.error && <p role="alert" className="my-2 break-words text-amber-300">{status.details.progress.error}</p>}
      <div className="my-3 flex flex-col gap-3 sm:flex-row sm:flex-wrap">
        {!status?.enabled ? <Button className={touch} variant="secondary" disabled={busy || !status?.releaseConfigured} onClick={() => void action("enable")}>{t("runtime.enable")}</Button> : <>
          <Button className={touch} variant="secondary" disabled={busy || installing} onClick={() => void action("plan")}>{t("runtime.install")}</Button>
          <Button className={touch} variant="ghost" disabled={busy || installing} onClick={() => void action("doctor")}>{t("runtime.doctor")}</Button>
          <Button className={touch} variant="ghost" disabled={busy || installing || !status.details?.previousId} onClick={() => void action("rollback")}>{t("runtime.rollback")}</Button>
          {installing && <Button className={touch} variant="ghost" disabled={busy} onClick={() => void action("cancel")}>{t("runtime.cancel")}</Button>}
        </>}
      </div>
      {plan && <div className="my-3 space-y-3 border border-[#3f476c] p-3">
        <p>{t("runtime.approval", { download: mib(plan.manifest.downloadBytes), disk: mib(plan.requiredBytes) })}</p>
        <Button className={touch} variant="secondary" disabled={busy || installing} onClick={() => void action("install")}>{t("runtime.approve")}</Button>
        <Button className={touch} variant="ghost" onClick={() => setPlan(null)}>{t("runtime.cancel")}</Button>
      </div>}
      {status?.enabled && <div className="my-3 flex min-w-0 flex-col gap-3">
        <input value={path} onChange={(event) => setPath(event.target.value)} autoCapitalize="none" autoCorrect="off" spellCheck={false} placeholder={t("runtime.path")} aria-label={t("runtime.path")} className="min-h-11 min-w-0 border border-[#3f476c] bg-[#0b0f23] px-3 py-2 text-base text-white" />
        <Button className={touch} variant="secondary" disabled={busy || !isRemoteExecutablePath(path)} onClick={() => void action("register")}>{t("runtime.register")}</Button>
      </div>}
      {status?.details?.apps.map((app) => <div key={app.id} className="flex min-w-0 flex-col gap-3 py-2 sm:flex-row sm:items-center sm:justify-between">
        <span className="min-w-0 break-words">{app.name}</span>
        <Button className={touch} variant="ghost" disabled={busy} onClick={() => void action("shortcut", app.id)}>{t("runtime.shortcut")}</Button>
      </div>)}
      {diagnostics != null && <pre className="mt-3 max-h-64 max-w-full overflow-auto whitespace-pre-wrap break-words text-xs">{JSON.stringify(diagnostics, null, 2)}</pre>}
    </details>
  );
}
