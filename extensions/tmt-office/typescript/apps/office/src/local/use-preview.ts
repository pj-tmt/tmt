import { useEffect, useState } from 'react';
import type { LocalRuntime } from './local-runtime.js';

type PreviewState<T> = { kind: 'loading' } | { kind: 'ready'; value: T } | { kind: 'failed' };

interface StoredPreview<T> {
  runtime: LocalRuntime | undefined;
  previewId: string;
  state: PreviewState<T>;
}

export function usePreview<T>(
  runtime: LocalRuntime | undefined,
  previewId: string,
  request: (runtime: LocalRuntime, previewId: string) => Promise<T>
): PreviewState<T> {
  const [stored, setStored] = useState<StoredPreview<T>>({
    runtime,
    previewId,
    state: { kind: 'loading' },
  });
  useEffect(() => {
    let active = true;
    setStored({ runtime, previewId, state: { kind: 'loading' } });
    if (!runtime) {
      setStored({ runtime, previewId, state: { kind: 'failed' } });
      return () => {
        active = false;
      };
    }
    void request(runtime, previewId).then(
      (value) => {
        if (active) setStored({ runtime, previewId, state: { kind: 'ready', value } });
      },
      () => {
        if (active) setStored({ runtime, previewId, state: { kind: 'failed' } });
      }
    );
    return () => {
      active = false;
    };
  }, [previewId, request, runtime]);
  return stored.runtime === runtime && stored.previewId === previewId
    ? stored.state
    : { kind: 'loading' };
}
