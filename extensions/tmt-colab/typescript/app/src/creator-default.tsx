import { BrowserToggle } from '@tmt/browser-ui/react';
import {
  createContext,
  useContext,
  useEffect,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from 'react';
import { text } from './strings.js';

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
const compactQuery = '(max-width: 640px)';
function subscribeCompact(changed: () => void) {
  const media = matchMedia(compactQuery);
  media.addEventListener('change', changed);
  return () => media.removeEventListener('change', changed);
}

/** The leaf owns its toggle-button semantics; Colab owns the responsive recipient label. */
export function CreatorToggle({ name, preference }: { name: string; preference: Preference }) {
  const compact = useSyncExternalStore(subscribeCompact, () => matchMedia(compactQuery).matches);
  const full = text.messageCreatorToggle(name);
  const [host, setHost] = useState<HTMLSpanElement | null>(null);
  useEffect(() => {
    // BrowserToggle has one label port. The local wrapper keeps its full accessible
    // name while the visible label compacts; no leaf interaction or role is replaced.
    host?.querySelector('button')?.setAttribute('aria-label', full);
  }, [host, full]);
  return (
    <span className="creator-toggle" ref={setHost}>
      <BrowserToggle
        pressed={preference.enabled}
        label={compact ? `@${name}` : full}
        onActivate={(event) => {
          if (event.isTrusted) preference.toggle();
        }}
      />
    </span>
  );
}
