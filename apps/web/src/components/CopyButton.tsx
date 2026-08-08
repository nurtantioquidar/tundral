"use client";

import { useRef, useState } from "react";

/* Copy-to-clipboard: label swap is the feedback. No toast. */
export function CopyButton({ value, label = "Copy" }: { value: string; label?: string }) {
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setState("copied");
    } catch {
      setState("failed");
    }
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setState("idle"), 2500);
  };

  return (
    <button type="button" className="copy-btn" data-state={state === "copied" ? "copied" : undefined} onClick={copy}>
      {state === "copied" ? "✓ Copied" : state === "failed" ? "Select + copy manually" : label}
    </button>
  );
}
