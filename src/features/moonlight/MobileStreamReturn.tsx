import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Button } from "../../components/ui/Button";
import {
  moonlightDisconnectStream,
  moonlightGetClipboardFromRemote,
  moonlightPresentStream,
  moonlightSendClipboardToRemote,
} from "../../lib/backend";
import { translate, translateSource } from "../../lib/i18n";
import { isAndroid } from "../../lib/platform";

type Action = "return" | "send" | "get" | "disconnect";
type Statistics = { renderedFpsX100: number; averageDecodePipelineUs: number };

function failureMessage(failure: unknown): string {
  if (failure && typeof failure === "object") {
    const details = failure as { details?: unknown; message?: unknown };
    if (typeof details.details === "string") return details.details;
    if (typeof details.message === "string") return details.message;
  }
  return String(failure);
}

export function MobileStreamReturn() {
  const [error, setError] = useState<string | null>(null);
  const [action, setAction] = useState<Action | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [streamStats, setStreamStats] = useState<Statistics | null>(null);
  const mounted = useRef(false);
  const busy = useRef(false);

  useEffect(() => {
    mounted.current = true;
    let active = true;
    let dispose: (() => void) | undefined;
    void listen<Statistics>("moonlight://statistics", ({ payload }) => {
      if (active) setStreamStats(payload);
    }).then((unlisten) => {
      if (active) dispose = unlisten;
      else unlisten();
    }).catch((failure) => {
      if (active) setError(failureMessage(failure));
    });
    return () => {
      active = false;
      mounted.current = false;
      dispose?.();
    };
  }, []);

  async function perform(next: Action) {
    if (busy.current) return;
    busy.current = true;
    setAction(next);
    setError(null);
    try {
      if (next === "return") await moonlightPresentStream();
      else if (next === "disconnect") await moonlightDisconnectStream();
      else {
        const result = next === "send"
          ? await moonlightSendClipboardToRemote()
          : await moonlightGetClipboardFromRemote();
        if (mounted.current) setStatus(translate(next === "send" ? "stream.clipboard.sent" : "stream.clipboard.received", {
          count: result.byteCount,
        }));
      }
    } catch (failure) {
      if (mounted.current) setError(failureMessage(failure));
    } finally {
      busy.current = false;
      if (mounted.current) setAction(null);
    }
  }

  return (
    <div className="sticky top-0 z-40 border-b border-[#3e4270] bg-[#10122af2] p-3 backdrop-blur">
      <div className="flex flex-wrap gap-2">
        <Button onClick={() => void perform("return")} disabled={action !== null}>
          {translate("mobile.returnToStream")}
        </Button>
        {!isAndroid && <>
          <Button variant="ghost" onClick={() => void perform("send")} disabled={action !== null}>
            {translate("generated.639d984d9ffc0361")}
          </Button>
          <Button variant="ghost" onClick={() => void perform("get")} disabled={action !== null}>
            {translate("generated.9d060e4dd135b141")}
          </Button>
        </>}
        <Button variant="danger" onClick={() => void perform("disconnect")} disabled={action !== null}>
          {action === "disconnect" ? translate("generated.6feadc6d3b71fbcc") : translate("generated.23c13af33709b5e3")}
        </Button>
      </div>
      {streamStats && (
        <p className="mt-2 text-sm text-[#9fb3cd]">
          {translate("mobile.streamStats", {
            fps: (streamStats.renderedFpsX100 / 100).toFixed(1),
            decode: (streamStats.averageDecodePipelineUs / 1000).toFixed(2),
          })}
        </p>
      )}
      {status && <p role="status" className="mt-2 text-[#7cff47]">{status}</p>}
      {error && <p role="alert" className="mt-2 text-[#ff687d]">{translateSource(error)}</p>}
    </div>
  );
}
