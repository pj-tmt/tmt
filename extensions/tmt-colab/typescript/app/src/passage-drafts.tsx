import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
} from 'react';
import type { PageTransport } from './transport.js';

// Only parent UI state lives here: no source, connection, record store or persistence. A mounted
// transport bounds the lifetime; leaving Today (including its held overlay copy) cannot erase drafts.
const memories = new WeakMap<PageTransport, Map<string, unknown>>();
const DraftScope = createContext<{ pageId: string; values: Map<string, unknown> } | undefined>(
  undefined,
);

export function PassageDraftScope({
  transport,
  pageId,
  children,
}: {
  transport: PageTransport;
  pageId: string;
  children?: ReactNode;
}) {
  const scope = useMemo(() => {
    let values = memories.get(transport);
    if (!values) {
      values = new Map();
      memories.set(transport, values);
    }
    return { pageId, values };
  }, [transport, pageId]);
  return <DraftScope value={scope}>{children}</DraftScope>;
}

/** The same controls own their drafts and publication fences. This retains their state across
 * presentation unmounts; restoring a value never invokes an action or changes admission. */
export function usePassageDraft<T>(id: string, initial: T): [T, Dispatch<SetStateAction<T>>] {
  const scope = useContext(DraftScope);
  const key = `${scope?.pageId ?? ''}:${id}`;
  const store = scope?.values;
  const read = () => (store?.has(key) ? (store.get(key) as T) : initial);
  const [held, setHeld] = useState(() => ({ key, store, value: read() }));
  let value = held.value;
  if (held.key !== key || held.store !== store) {
    value = read();
    setHeld({ key, store, value });
  }
  const current = useRef(value);
  current.current = value;
  const update: Dispatch<SetStateAction<T>> = useCallback(
    (action) => {
      const next =
        typeof action === 'function' ? (action as (previous: T) => T)(current.current) : action;
      if (Object.is(next, current.current)) return;
      current.current = next;
      store?.set(key, next);
      setHeld({ key, store, value: next });
    },
    [key, store],
  );
  return [value, update];
}
