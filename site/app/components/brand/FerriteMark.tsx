import type { CSSProperties } from 'react';
import { MARK } from './geometry';
export type MarkVariant = 'primary' | 'inverse' | 'mono';
export interface FerriteMarkProps {
  size?: number | string;
  variant?: MarkVariant;
  decorative?: boolean;
  className?: string;
}
/** Every live logo instance shares the same authored vector geometry. */
export function FerriteMark({ size = 24, variant = 'primary', decorative = false, className = '' }: FerriteMarkProps) {
  return <svg xmlns="http://www.w3.org/2000/svg" viewBox={MARK.viewBox} width={size} height={size}
    className={`ferrite-mark ferrite-mark--${variant} ${className}`} style={{ '--mark-size': typeof size === 'number' ? `${size}px` : size } as CSSProperties}
    role={decorative ? undefined : 'img'} aria-hidden={decorative || undefined} aria-label={decorative ? undefined : 'Ferrite Lithic — Verified Edge'} focusable="false">
    <path className="ferrite-mark__branch" d={MARK.branch} fill="none" stroke="currentColor" strokeWidth="2" strokeLinejoin="miter" />
    <rect className="ferrite-mark__input" {...MARK.input} />
    <rect className="ferrite-mark__upper" {...MARK.upper} />
    <rect className="ferrite-mark__lower" {...MARK.lower} fill="currentColor" />
  </svg>;
}
