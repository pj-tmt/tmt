import { createContext, useContext, useState, type ReactNode } from 'react';

type Preference = { enabled: boolean; revision: number; toggle(): void };
const CreatorDefault = createContext<Preference | undefined>(undefined);
const prefix = 'tmt-colab-creator-default-v1:';

/** Convenience only: a refused storage access never grants or blocks a recipient. */
export function readCreatorDefault(scope: string): boolean {
  try {
    return localStorage.getItem(prefix + scope) !== 'off';
  } catch {
    return true;
  }
}
export function writeCreatorDefault(scope: string, enabled: boolean): void {
  try {
    localStorage.setItem(prefix + scope, enabled ? 'on' : 'off');
  } catch {
    /* The mounted page retains its preference even without device storage. */
  }
}

/** One preference for every composer on this page and viewer device. Key by scope at the caller. */
export function CreatorDefaultProvider({
  scope,
  children,
}: {
  scope: string;
  children: ReactNode;
}) {
  const [state, setState] = useState(() => ({ enabled: readCreatorDefault(scope), revision: 0 }));
  const preference = {
    ...state,
    toggle() {
      const enabled = !state.enabled;
      writeCreatorDefault(scope, enabled);
      setState({ enabled, revision: state.revision + 1 });
    },
  };
  return <CreatorDefault value={preference}>{children}</CreatorDefault>;
}
export function useCreatorDefault(): Preference {
  const shared = useContext(CreatorDefault);
  // Standalone composers retain the same behavior without inventing a page/storage scope.
  const [local, setLocal] = useState({ enabled: true, revision: 0 });
  return (
    shared ?? {
      ...local,
      toggle: () => setLocal({ enabled: !local.enabled, revision: local.revision + 1 }),
    }
  );
}
