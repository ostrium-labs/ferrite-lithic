import { useEffect, useRef, useState, type ReactNode } from "react";

/** Copies rendered text, preserving authored whitespace and highlighted MDX children. */
export function CodeBlock({ children, label = "Code sample" }: { children: ReactNode; label?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const reset = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [status, setStatus] = useState<"Copy" | "Copied" | "Select code to copy">("Copy");
  useEffect(() => () => clearTimeout(reset.current), []);
  const copy = async () => {
    const pre = ref.current?.querySelector("pre");
    if (!pre) return;
    try {
      await navigator.clipboard.writeText(pre.textContent ?? "");
      setStatus("Copied");
    } catch {
      const selection = window.getSelection();
      const range = document.createRange();
      range.selectNodeContents(pre);
      selection?.removeAllRanges();
      selection?.addRange(range);
      setStatus("Select code to copy");
    }
    clearTimeout(reset.current);
    reset.current = setTimeout(() => setStatus("Copy"), 2200);
  };
  return <div className="code-surface" ref={ref}>
    <div className="code-surface__head"><span>{label}</span><button type="button" onClick={copy} aria-label={`Copy ${label.toLowerCase()}`}>{status}</button></div>
    {children}
    <span className="sr-only" role="status">{status === "Copy" ? "" : status}</span>
  </div>;
}
