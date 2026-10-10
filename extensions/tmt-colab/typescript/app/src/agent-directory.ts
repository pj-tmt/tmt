import { createContext, useEffect, useState } from 'react';
import { ReadRefusedError, SessionEndedError } from './ask-remote.js';
import type { AgentDestination } from './live-ask.js';
import type { AskBinding } from './ask-panel.js';
import type { RunningDriver } from './ask-remote.js';

/** What a composer knows about the agents it can ask. `ready` with no agents is a real answer;
 * `loading` and `failed` are not, and the composer must not offer an Ask for either. A failure
 * carries Remote's refusal code when Remote refused the read, as a reference for the reader. */
export type AgentDirectory =
  | { state: 'loading' }
  | { state: 'failed'; code?: string }
  | { state: 'ready'; agents: AgentDestination[] };

const LOADING: AgentDirectory = { state: 'loading' };

/** Discovery belongs to the binding. A new binding (Reconnect replaced the Ask facade) or a
 * retry starts a new read and shows `loading` in the same render, so no list from the old
 * binding can enable an Ask; a read that settles for a replaced binding or attempt is ignored. */
export function useAgentDirectory(
  binding: { destinations(): Promise<AgentDestination[]> } | undefined,
): { directory: AgentDirectory; retry(): void } {
  const [attempt, setAttempt] = useState(0);
  const [settled, setSettled] = useState<{
    binding: unknown;
    attempt: number;
    directory: AgentDirectory;
  }>();
  useEffect(() => {
    if (!binding) return;
    let active = true;
    const settle = (directory: AgentDirectory) => {
      if (active) setSettled({ binding, attempt, directory });
    };
    void binding.destinations().then(
      (agents) => settle({ state: 'ready', agents }),
      (error: unknown) =>
        settle({
          state: 'failed',
          ...(error instanceof ReadRefusedError || error instanceof SessionEndedError
            ? { code: error.code }
            : {}),
        }),
    );
    return () => {
      active = false;
    };
  }, [binding, attempt]);
  const current =
    binding !== undefined && settled?.binding === binding && settled.attempt === attempt
      ? settled.directory
      : LOADING;
  return { directory: current, retry: () => setAttempt((n) => n + 1) };
}

/** Read-only presentation scope, distinct from the composer's destination admission. */
export const AgentPresentation = createContext<readonly AgentDestination[]>([]);

export function runningDriverFor(
  rows: readonly AgentDestination[],
  machine: string,
  agent: string,
): RunningDriver | undefined {
  const matches = rows.filter((row) => row.machine === machine && row.agent === agent);
  const value = matches.length === 1 ? matches[0]!.runningDriver : undefined;
  return value === 'claude' || value === 'codex' ? value : undefined;
}

/** Page/binding, admission and new-operation changes invalidate the previous read immediately.
 * No polling, retained driver history, recovery or Ask admission occurs here. */
export function useAgentPresentation(
  binding: Pick<AskBinding, 'observeDestinations'> | undefined,
  page: string,
  admitted: boolean,
  operations: string,
): readonly AgentDestination[] {
  const [settled, setSettled] = useState<{
    binding: unknown;
    page: string;
    operations: string;
    agents: AgentDestination[];
  }>();
  // Discard a replaced scope, even when the same binding or operation set later returns.
  if (
    settled &&
    (!admitted ||
      settled.binding !== binding ||
      settled.page !== page ||
      settled.operations !== operations)
  )
    setSettled(undefined);
  useEffect(() => {
    if (!admitted || !binding?.observeDestinations) return;
    let active = true;
    const settle = (agents: AgentDestination[]) => {
      if (active) setSettled({ binding, page, operations, agents });
    };
    void binding.observeDestinations().then(
      (observation) => settle(observation.kind === 'ready' ? observation.destinations : []),
      () => settle([]),
    );
    return () => {
      active = false;
    };
  }, [binding, page, admitted, operations]);
  return admitted &&
    settled &&
    settled.binding === binding &&
    settled.page === page &&
    settled.operations === operations
    ? settled.agents
    : [];
}
