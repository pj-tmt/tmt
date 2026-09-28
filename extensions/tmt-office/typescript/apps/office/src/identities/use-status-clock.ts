import { useEffect, useState } from 'react';
import { isStatusFresh } from './identity-status.js';
import type { IdentityStatusSnapshot } from './identity-status.js';

/** One deadline for the observed directory; never poll the network or animate actors. */
export function useStatusClock(
  profiles: readonly { selfReportedStatus: IdentityStatusSnapshot | null }[]
) {
  const [nowMs, setNowMs] = useState(Date.now);
  useEffect(() => {
    let deadline: ReturnType<typeof setTimeout> | undefined;
    function sample() {
      clearTimeout(deadline);
      const now = Date.now();
      setNowMs(now);
      const next = profiles.reduce(
        (earliest, { selfReportedStatus: status }) =>
          status && isStatusFresh(status, now) ? Math.min(earliest, status.expiresAtMs) : earliest,
        Infinity
      );
      deadline = Number.isFinite(next) ? setTimeout(sample, next - now) : undefined;
    }
    function visibility() {
      if (document.visibilityState === 'visible') sample();
    }
    sample();
    document.addEventListener('visibilitychange', visibility);
    return () => {
      clearTimeout(deadline);
      document.removeEventListener('visibilitychange', visibility);
    };
  }, [profiles]);
  return nowMs;
}
