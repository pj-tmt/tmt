import { generatedId, requireValue } from '@tmt/colab-client';

/** Remote supplies the internal mount; public location carries only a display target. */
export function publicEntry(path = location.pathname): boolean {
  return (
    path === '/colab' || path === '/colab/' || /^\/(?:p|read)\/[A-Za-z0-9_-]{4,64}$/.test(path)
  );
}
export function entryMount(): URL {
  const supplied = document.querySelector<HTMLMetaElement>('meta[name="tmt-colab-mount"]')?.content;
  const url = new URL(supplied ?? location.pathname, location.origin);
  if (
    url.origin !== location.origin ||
    !/^\/r\/[a-z2-7]{16}\/x\/colab\/$/.test(url.pathname) ||
    url.search ||
    url.hash
  )
    throw new Error('Invalid Colab mount');
  return url;
}

/** Public owner fragments are display-only thread targets, never space or reader authority. */
export function entryThread(hash = location.hash): string | null {
  requireValue(hash === '' || hash.startsWith('#'));
  const target = new URLSearchParams(hash.slice(1));
  requireValue([...target.keys()].every((key) => key === 't') && target.getAll('t').length <= 1);
  if (!target.has('t')) return null;
  const thread = target.get('t')!;
  generatedId(thread);
  return thread;
}
