import { useEffect, useState } from 'react';
import type { AgentDestination } from './live-ask.js';

/** What a composer knows about the agents it can ask. `ready` with no agents is a real answer;
 * `loading` and `failed` are not, and the composer must not offer an Ask for either. */
export type AgentDirectory =
  | { state: 'loading' }
  | { state: 'failed' }
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
      () => settle({ state: 'failed' }),
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
