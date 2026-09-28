import { act, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { LocalRuntime } from './local-runtime.js';
import { usePreview } from './use-preview.js';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: () => void;
  const promise = new Promise<T>((accept, decline) => {
    resolve = accept;
    reject = decline;
  });
  return { promise, resolve, reject };
}

const request = (runtime: LocalRuntime, id: string) => runtime.avatarPreview(id);
function Harness({ runtime, id }: { runtime: LocalRuntime; id: string }) {
  const state = usePreview(runtime, id, request);
  return <p>{state.kind === 'ready' ? state.value.digest : state.kind}</p>;
}

describe('preview request lifecycle', () => {
  it('resets failure and fences an older completion after the key changes', async () => {
    const old = deferred<{ digest: string; pack: never }>();
    const next = deferred<{ digest: string; pack: never }>();
    const runtime = {
      avatarPreview: vi.fn((id: string) => (id === 'old' ? old.promise : next.promise)),
    } as unknown as LocalRuntime;
    const view = render(<Harness runtime={runtime} id="old" />);
    old.reject();
    expect(await screen.findByText('failed')).toBeTruthy();
    view.rerender(<Harness runtime={runtime} id="next" />);
    expect(screen.getByText('loading')).toBeTruthy();
    next.resolve({ digest: 'next-digest', pack: undefined as never });
    expect(await screen.findByText('next-digest')).toBeTruthy();

    const late = deferred<{ digest: string; pack: never }>();
    const latest = deferred<{ digest: string; pack: never }>();
    runtime.avatarPreview = vi.fn((id: string) => (id === 'late' ? late.promise : latest.promise));
    view.rerender(<Harness runtime={runtime} id="late" />);
    view.rerender(<Harness runtime={runtime} id="latest" />);
    latest.resolve({ digest: 'latest-digest', pack: undefined as never });
    expect(await screen.findByText('latest-digest')).toBeTruthy();
    await act(async () => {
      late.resolve({ digest: 'stale-digest', pack: undefined as never });
    });
    expect(screen.getByText('latest-digest')).toBeTruthy();
    expect(screen.queryByText('stale-digest')).toBeNull();
  });
});
