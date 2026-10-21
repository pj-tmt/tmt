import { useEffect, useState } from 'react';
import { createBusyMarkGate } from './busy-mark-gate';

export { busyMarkDelayMs, busyMarkHoldMs, createBusyMarkGate } from './busy-mark-gate';

/** Whether a busy action shows its mark now, under the delay and hold. */
export function useBusyMark(busy: boolean): boolean {
  const [shown, setShown] = useState(false);
  const [gate] = useState(() => createBusyMarkGate(setShown));
  useEffect(() => {
    gate.set(busy);
  }, [gate, busy]);
  useEffect(() => () => gate.dispose(), [gate]);
  return shown;
}
