import { FerriteMark, type MarkVariant } from './FerriteMark';
export function FerriteLogo({ variant = 'primary', stacked = false, className = '' }: { variant?: MarkVariant; stacked?: boolean; className?: string }) {
  return <span className={`ferrite-logo ferrite-logo--${variant}${stacked ? ' ferrite-logo--stacked' : ''} ${className}`}>
    <FerriteMark decorative variant={variant} size="1.75rem" /><span className="ferrite-logo__wordmark">ferrite-lithic</span>
  </span>;
}
