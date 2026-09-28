import { useRef, useState } from 'react';

/** Only the current field receives async clipboard feedback or the manual fallback. */
export function useCopyReference(reference: string, reveal?: () => void) {
  const field = useRef<HTMLInputElement>(null);
  const [message, setMessage] = useState('');
  async function copy() {
    const target = field.current;
    try {
      await navigator.clipboard.writeText(reference);
      if (target && field.current === target && target.value === reference)
        setMessage('Reference copied.');
    } catch {
      if (target && field.current === target && target.value === reference) {
        reveal?.();
        target.focus();
        target.select();
        setMessage('Clipboard unavailable. Copy the selected reference manually.');
      }
    }
  }
  return { field, message, copy };
}
