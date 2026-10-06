import type { ReactNode } from "react";

/** A named instrument with a readable coordinate space, including on a phone. */
export function Instrument({ title, description, children, controls, compact = false }: {
  title: string;
  description: string;
  children: ReactNode;
  controls?: ReactNode;
  compact?: boolean;
}) {
  return <div className={`instrument-frame${compact ? " instrument-frame--compact" : ""}`}>
    <div className="instrument-frame__head">
      <span>{title}</span>
      {controls}
    </div>
    <div className="instrument-scroll" role="region" aria-label={description} tabIndex={0}>
      {children}
    </div>
    <p className="instrument-frame__hint">Scroll the trace horizontally when it extends beyond the screen.</p>
  </div>;
}
