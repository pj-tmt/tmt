import { useEffect, useState } from 'react';
import { text } from './strings.js';

/** A wait shorter than this shows nothing: a normal open never flashes the cue. */
export const READING_CONNECTING_MS = 400;

/**
 * The cue beside a disabled `Mark read` while the page cannot act yet: a muted `Connecting…` on
 * the same row, after the wait passes 400 ms, so nothing below the band moves. The live region
 * exists as soon as the wait starts so its text is announced when it appears; assistive text says
 * what the wait is for.
 */
export function ReadingConnecting({ waiting }: { waiting: boolean }) {
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    if (!waiting) {
      setSlow(false);
      return;
    }
    const timer = setTimeout(() => setSlow(true), READING_CONNECTING_MS);
    return () => clearTimeout(timer);
  }, [waiting]);
  if (!waiting) return null;
  return (
    <span className="reading-connecting" role="status">
      {slow && (
        <>
          <span aria-hidden="true">{text.readingConnecting}</span>
          <span className="reading-connecting-text">{text.readingNotReady}</span>
        </>
      )}
    </span>
  );
}
