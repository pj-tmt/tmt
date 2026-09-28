import {
  createContext,
  useCallback,
  useContext,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import type { ReactNode } from 'react';

type Protection = 'ready' | 'draft' | 'pending';
interface Navigation {
  epoch: number;
  state: Protection;
  request(action: () => void): void;
  report(id: string, state: Protection): void;
}
const Context = createContext<Navigation | undefined>(undefined);

/** One leave gate for board categories, threads and spatial entry points. */
export function BoardNavigation({ children }: { children: ReactNode }) {
  const states = useRef(new Map<string, Protection>());
  const [state, setState] = useState<Protection>('ready');
  const [epoch, setEpoch] = useState(0);
  const [requested, setRequested] = useState<{ action(): void }>();
  const current = useCallback((): Protection => {
    const values = [...states.current.values()];
    return values.includes('pending') ? 'pending' : values.includes('draft') ? 'draft' : 'ready';
  }, []);
  const report = useCallback(
    (id: string, value: Protection) => {
      if (value === 'ready') states.current.delete(id);
      else states.current.set(id, value);
      setState(current());
    },
    [current]
  );
  const request = useCallback(
    (action: () => void) => {
      if (current() !== 'ready') setRequested({ action });
      else {
        setRequested(undefined);
        action();
      }
    },
    [current]
  );
  return (
    <Context.Provider value={{ epoch, state, request, report }}>
      {requested && (
        <section aria-label="Switch discussion confirmation">
          <h2>Leave this discussion?</h2>
          <p>
            {state === 'pending'
              ? 'Resolve or explicitly discard the unconfirmed operation before switching. It may already have been applied.'
              : 'Switching discards unsaved drafts here. Saved discussions are not deleted.'}
          </p>
          <button
            disabled={state === 'pending'}
            onClick={() => {
              if (current() === 'pending') return;
              setEpoch((value) => value + 1);
              setRequested(undefined);
              requested.action();
            }}
          >
            Discard drafts and switch
          </button>
          <button onClick={() => setRequested(undefined)}>Keep this discussion</button>
        </section>
      )}
      {children}
    </Context.Provider>
  );
}

export function useBoardNavigation(): Navigation {
  const navigation = useContext(Context);
  if (!navigation) throw new Error('Board navigation owner is missing.');
  return navigation;
}

/** Forms own their data; this aggregate contains only derived leave protection. */
export function useBoardProtection(state: Protection) {
  const navigation = useContext(Context);
  const id = useId();
  const report = navigation?.report;
  useLayoutEffect(() => {
    report?.(id, state);
  }, [id, report, state]);
  useLayoutEffect(() => () => report?.(id, 'ready'), [id, report]);
}
