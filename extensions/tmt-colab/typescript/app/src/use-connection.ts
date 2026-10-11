import { useEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import { ConnectionController, type ConnectionControllerOptions } from './connection-controller.js';
import type { ConnectionState } from './connection-model.js';
import { ReadmitBudget } from './readmit.js';

interface Options<Page> {
  /** The page the route loaded: a controller lives exactly as long as this does. */
  loaded: Page;
  /** The current binding, which a reopen replaces. */
  binding: ConnectionControllerOptions<Page>['binding'];
  reopen(signal: AbortSignal): Promise<Page>;
  opened(next: Page): void;
  failed(error: Error): void;
}

/**
 * The one connection state of an open page (#2572): `connected | reconnecting | ended`. The page
 * reports each failure its binding publishes with `controller.report` and calls
 * `controller.reset` when it takes a new binding; every reader reads `state`.
 */
export function useConnection<Page>(options: Options<Page>): {
  state: ConnectionState;
  controller: ConnectionController<Page>;
} {
  const latest = useRef(options);
  latest.current = options;
  // Automatic reopens are budgeted per view, across route reloads of the same page.
  const budget = useRef(new ReadmitBudget());
  const controller = useMemo(
    () =>
      new ConnectionController<Page>({
        binding: () => latest.current.binding(),
        reopen: (signal) => latest.current.reopen(signal),
        opened: (next) => latest.current.opened(next),
        failed: (error) => latest.current.failed(error),
        budget: budget.current,
      }),
    // A route reload brings its own binding: attempts in flight and a reopened binding end with it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [options.loaded],
  );
  useEffect(() => () => controller.dispose(), [controller]);
  const state = useSyncExternalStore(controller.subscribe, () => controller.state);
  return { state, controller };
}
