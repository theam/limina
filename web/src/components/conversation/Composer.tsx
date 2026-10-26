import { ArrowUp, StopFilledAlt } from "@carbon/icons-react";
import * as Tooltip from "@radix-ui/react-tooltip";
import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import type { GuidanceKind } from "../../lib/types";

export interface ComposerHandle {
  focus: () => void;
  insert: (text: string) => void;
}

const KINDS: { value: GuidanceKind; label: string; hint: string }[] = [
  { value: "STEER", label: "Steer", hint: "Redirect or add context to the work" },
  { value: "ANSWER", label: "Answer", hint: "Reply to a question Limina asked" },
  { value: "APPROVAL", label: "Approve", hint: "Approve a proposed decision or action" },
];

interface ComposerProps {
  placeholder: string;
  defaultKind: GuidanceKind;
  running: boolean;
  disabled?: boolean;
  onSend: (body: string, kind: GuidanceKind) => void;
  onInterrupt: () => void;
  interrupting?: boolean;
}

export const Composer = forwardRef<ComposerHandle, ComposerProps>(function Composer(
  { placeholder, defaultKind, running, disabled, onSend, onInterrupt, interrupting },
  ref,
) {
  const [body, setBody] = useState("");
  const [kind, setKind] = useState<GuidanceKind>(defaultKind);
  const textarea = useRef<HTMLTextAreaElement>(null);
  useImperativeHandle(ref, () => ({ focus: () => textarea.current?.focus(), insert: (text: string) => { setBody((current) => current ? `${current}\n${text}` : text); textarea.current?.focus(); } }));

  useEffect(() => setKind(defaultKind), [defaultKind]);

  // Grow with content up to a comfortable limit, like a native message field.
  useEffect(() => {
    const element = textarea.current;
    if (!element) return;
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, Math.round(window.innerHeight * 0.4))}px`;
  }, [body]);

  const send = () => {
    const value = body.trim();
    if (!value || disabled) return;
    onSend(value, kind);
    setBody("");
    if (kind !== defaultKind) setKind(defaultKind);
  };

  return (
    <form
      className="composer"
      data-disabled={disabled || undefined}
      onSubmit={(event) => {
        event.preventDefault();
        send();
      }}
    >
      <textarea
        ref={textarea}
        className="composer-input"
        rows={1}
        value={body}
        disabled={disabled}
        placeholder={placeholder}
        aria-label="Message to Limina"
        onChange={(event) => setBody(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
            event.preventDefault();
            send();
          }
        }}
      />
      <div className="composer-bar">
        <div className="segmented" role="group" aria-label="Message type">
          {KINDS.map((item) => (
            <Tooltip.Root key={item.value}>
              <Tooltip.Trigger asChild>
                <button type="button" aria-pressed={kind === item.value} onClick={() => setKind(item.value)} disabled={disabled}>
                  {item.label}
                </button>
              </Tooltip.Trigger>
              <Tooltip.Portal>
                <Tooltip.Content className="tooltip" sideOffset={6}>
                  {item.hint}
                </Tooltip.Content>
              </Tooltip.Portal>
            </Tooltip.Root>
          ))}
        </div>
        <span className="composer-hint">
          <kbd className="kbd">↵</kbd> send <kbd className="kbd">⇧↵</kbd> new line
        </span>
        <span className="spacer" />
        {running && (
          <Tooltip.Root>
            <Tooltip.Trigger asChild>
              <button type="button" className="btn btn-sm" onClick={onInterrupt} disabled={interrupting}>
                <StopFilledAlt size={14} /> Interrupt
              </button>
            </Tooltip.Trigger>
            <Tooltip.Portal>
              <Tooltip.Content className="tooltip" sideOffset={6}>
                Stop the current turn and pause the project
              </Tooltip.Content>
            </Tooltip.Portal>
          </Tooltip.Root>
        )}
        <button
          type="submit"
          className="btn btn-primary btn-sm btn-icon composer-send"
          disabled={!body.trim() || disabled}
          aria-label="Send"
        >
          <ArrowUp size={16} />
        </button>
      </div>
    </form>
  );
});
