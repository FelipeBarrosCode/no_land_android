import { translate } from "../../lib/i18n";
import { useEffect, useId, useRef, type ReactNode } from "react";
import { Button } from "../../components/ui/Button";
import { ModalBody, ModalFrame } from "../../components/ui/ModalFrame";

const dialogStack: symbol[] = [];

export function FeedbackDialog({ title, onClose, children }: { title: string; onClose: () => void; children: ReactNode }) {
  const titleId = useId();
  const root = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  close.current = onClose;

  useEffect(() => {
    const token = Symbol("feedback-dialog");
    dialogStack.push(token);
    const previous = document.activeElement;
    const focusable = () => Array.from(root.current?.querySelectorAll<HTMLElement>(
      'button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex="0"]',
    ) ?? []).filter((element) => element.getClientRects().length > 0 && !element.closest('[hidden], [aria-hidden="true"]'));
    focusable()[0]?.focus();
    const keydown = (event: KeyboardEvent) => {
      if (dialogStack[dialogStack.length - 1] !== token) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        close.current();
      } else if (event.key === "Tab") {
        const elements = focusable();
        const first = elements[0];
        const last = elements[elements.length - 1];
        if (!first) { event.preventDefault(); root.current?.focus(); return; }
        if (event.shiftKey && (document.activeElement === first || !root.current?.contains(document.activeElement))) {
          event.preventDefault(); last.focus();
        } else if (!event.shiftKey && (document.activeElement === last || !root.current?.contains(document.activeElement))) {
          event.preventDefault(); first.focus();
        }
      }
    };
    // Capture prevents Escape from also dismissing the underlying server picker.
    window.addEventListener("keydown", keydown, true);
    return () => {
      dialogStack.splice(dialogStack.indexOf(token), 1);
      window.removeEventListener("keydown", keydown, true);
      if (previous instanceof HTMLElement && previous.isConnected && !previous.closest('[aria-hidden="true"], [inert]')) previous.focus();
    };
  }, []);

  return (
    <ModalFrame panelClassName="glass-panel pixel-frame max-w-2xl" labelledBy={titleId} zIndexClassName="z-[110]">
      <div ref={root} tabIndex={-1} className="feedback-ui flex min-h-0 flex-1 flex-col">
        <header className="flex shrink-0 items-center justify-between gap-3 border-b border-[#495188] p-4">
          <h2 id={titleId} className="text-lg font-semibold text-white">{title}</h2>
          <Button type="button" variant="ghost" aria-label={translate("feedback.31225400a92b9e0f", { value0: title })} onClick={onClose}>{translate("common.close")}</Button>
        </header>
        <ModalBody className="space-y-4 p-4 text-[#d3e4f6]">{children}</ModalBody>
      </div>
    </ModalFrame>
  );
}
