import { FEEDBACK_BATCH_SIZE, FEEDBACK_CACHE_TTL_MS, providerDecimalId } from "./model";
import type { FeedbackMachineSummary } from "./types";

type Entry = { summary: FeedbackMachineSummary; expiresAt: number };
type Loader = (ids: string[]) => Promise<FeedbackMachineSummary[]>;

export class FeedbackSummaryCache {
  private scope: string | null = null;
  private generation = 0;
  private entries = new Map<string, Entry>();
  private pending = new Map<string, Promise<void>>();

  private load: Loader;
  private now: () => number;

  constructor(load: Loader, now: () => number = Date.now) {
    this.load = load;
    this.now = now;
  }

  setScope(scope: string | null) {
    if (scope !== this.scope) {
      this.scope = scope;
      this.invalidate();
    }
  }

  invalidate() {
    this.generation += 1;
    this.entries.clear();
    this.pending.clear();
  }

  peek(id: string): FeedbackMachineSummary | undefined {
    const entry = this.entries.get(id);
    if (!this.scope || !entry) return undefined;
    if (entry.expiresAt <= this.now()) {
      this.entries.delete(id);
      return undefined;
    }
    return entry.summary;
  }

  expiresIn(ids: readonly string[]): number {
    return Math.max(1, Math.min(FEEDBACK_CACHE_TTL_MS, ...ids.map((id) =>
      (this.entries.get(id)?.expiresAt ?? this.now() + FEEDBACK_CACHE_TTL_MS) - this.now())));
  }

  async read(ids: readonly string[]): Promise<Map<string, FeedbackMachineSummary>> {
    if (!this.scope) return new Map();
    const generation = this.generation;
    const unique = [...new Set(ids.filter((id) => providerDecimalId(id) !== null))];
    const missing = unique.filter((id) => !this.peek(id) && !this.pending.has(id));
    for (let offset = 0; offset < missing.length; offset += FEEDBACK_BATCH_SIZE) {
      const batch = missing.slice(offset, offset + FEEDBACK_BATCH_SIZE);
      const request = this.load(batch).then((summaries) => {
        if (generation !== this.generation || !this.scope) return;
        const expiresAt = this.now() + FEEDBACK_CACHE_TTL_MS;
        for (const summary of summaries) {
          if (batch.includes(summary.providerMachineId)) this.entries.set(summary.providerMachineId, { summary, expiresAt });
        }
        // Bound retained viewer data even when the user browses many pages.
        while (this.entries.size > 500) this.entries.delete(this.entries.keys().next().value as string);
      }).finally(() => {
        if (generation === this.generation) batch.forEach((id) => this.pending.delete(id));
      });
      batch.forEach((id) => this.pending.set(id, request));
    }
    const results = await Promise.allSettled([...new Set(unique.map((id) => this.pending.get(id)).filter((request) => request !== undefined))]);
    if (generation !== this.generation || !this.scope) return new Map();
    const failure = results.find((result) => result.status === "rejected");
    if (failure?.status === "rejected") throw failure.reason;
    return new Map(unique.flatMap((id) => {
      const summary = this.peek(id);
      return summary ? [[id, summary] as const] : [];
    }));
  }
}
