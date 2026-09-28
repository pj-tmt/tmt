import { useRef, useState } from 'react';
import { LocalHttpError } from './local-runtime.js';
import { useBoardProtection } from './board-navigation.js';

export function boardErrorMessage(error: unknown): string {
  if (error instanceof LocalHttpError) {
    if (error.code === 'BOARD_CURSOR_STALE') return 'The board changed. Refresh to continue.';
    if (error.code === 'BOARD_REVISION_CONFLICT')
      return 'This entry changed. Refresh it before editing again.';
    if (error.code === 'BOARD_FORBIDDEN') return 'This entry can only be changed by its author.';
  }
  return 'The board request could not be completed. Your draft is still here.';
}

/** One frozen operation per mounted form. Editing never silently abandons a retry. */
export function useBoardMutation<T>(dirty: boolean) {
  const operation = useRef<{ operationId: string; input: T } | undefined>(undefined);
  const active = useRef(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  useBoardProtection(busy || operation.current ? 'pending' : dirty ? 'draft' : 'ready');
  return {
    busy,
    locked: busy || Boolean(operation.current),
    error,
    input: operation.current?.input,
    uncertain: !busy && Boolean(operation.current),
    discard() {
      if (active.current) return;
      operation.current = undefined;
      setError(undefined);
    },
    async run<R>(
      input: T,
      send: (input: T & { operationId: string }) => Promise<R>,
      completed: (result: R) => void
    ) {
      if (active.current) return;
      active.current = true;
      operation.current ??= { input, operationId: crypto.randomUUID() };
      const pending = operation.current;
      setBusy(true);
      setError(undefined);
      try {
        const result = await send({ ...pending.input, operationId: pending.operationId });
        operation.current = undefined;
        completed(result);
      } catch (caught) {
        if (caught instanceof LocalHttpError && caught.status >= 400 && caught.status < 500)
          operation.current = undefined;
        setError(boardErrorMessage(caught));
      } finally {
        active.current = false;
        setBusy(false);
      }
    },
  };
}
