import type { RuntimePlan } from "../../lib/types";

export function isRuntimeInstalling(phase?: string): boolean {
  return ["checking", "downloading", "verifying", "extracting", "validating"].includes(phase ?? "");
}

export function isRemoteExecutablePath(path: string): boolean {
  return path.startsWith("/") && path.length > 1 && !/[\0\r\n]/.test(path);
}

export function parseRuntimePlan(value: unknown): RuntimePlan | null {
  if (!value || typeof value !== "object") return null;
  const plan = value as RuntimePlan;
  const manifest = plan.manifest;
  const size = (n: number) => Number.isSafeInteger(n) && n > 0;
  if (!manifest || typeof manifest.id !== "string" || !/^[A-Za-z0-9_-]{1,96}$/.test(manifest.id)
    || typeof manifest.sha256 !== "string" || !/^[a-f0-9]{64}$/i.test(manifest.sha256)
    || !size(plan.requiredBytes) || !size(manifest.downloadBytes) || !size(manifest.extractedBytes)
    || plan.requiredBytes < manifest.downloadBytes + manifest.extractedBytes) return null;
  return plan;
}

export function runtimeError(error: unknown): string {
  if (error && typeof error === "object" && "message" in error) {
    const e = error as { message: unknown; details?: unknown };
    return `${String(e.message)}${typeof e.details === "string" && e.details ? `: ${e.details}` : ""}`;
  }
  return String(error);
}

/** Visibility/expansion and request lifetime are separate from authorized remote
 * operations: stopping a poll never kills an installation or a running game. */
export function createRuntimePoller<T>(options: {
  fetch: () => Promise<T>;
  shouldPoll: () => boolean;
  onValue: (value: T) => void;
  onError: (error: unknown) => void;
}) {
  let stopped = false;
  let fetching = false;
  return {
    async tick() {
      if (stopped || fetching || !options.shouldPoll()) return;
      fetching = true;
      try { const value = await options.fetch(); if (!stopped && options.shouldPoll()) options.onValue(value); }
      catch (error) { if (!stopped && options.shouldPoll()) options.onError(error); }
      finally { fetching = false; }
    },
    stop() { stopped = true; },
  };
}
