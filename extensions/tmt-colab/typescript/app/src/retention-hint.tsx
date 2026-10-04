import { Clock } from 'lucide-react';
import { expiryText, localTime, type ExpiryInfo } from './expiry.js';

/** A page fact in trusted chrome; never an access or deletion decision. */
export function RetentionHint({ page }: { page: Partial<ExpiryInfo> }) {
  const warning = page.warnings?.some((w) => w === 'expires-soon' || w === 'expired');
  return (
    <p
      className={`retention-hint${warning ? ' retention-warning' : ''}`}
      title={page.expiresAtMs == null ? undefined : localTime(page.expiresAtMs)}
    >
      {warning && (
        <span className="retention-mark" aria-hidden="true">
          <Clock aria-hidden />
        </span>
      )}
      {expiryText(page)}
    </p>
  );
}
