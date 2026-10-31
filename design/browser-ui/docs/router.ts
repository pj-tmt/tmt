import { useSyncExternalStore, type MouseEvent } from 'react';

/** A history-backed path, so links inside the docs and the mock never reload the page. */
function subscribe(notify: () => void) {
  addEventListener('popstate', notify);
  return () => removeEventListener('popstate', notify);
}
export function navigate(href: string) {
  history.pushState(null, '', href);
  dispatchEvent(new PopStateEvent('popstate'));
  scrollTo(0, 0);
}
export function usePath() {
  return useSyncExternalStore(subscribe, () => location.pathname);
}
/** An in-app link's click handler; modified clicks keep the browser's own behaviour. */
export function follow(event: MouseEvent<HTMLAnchorElement>) {
  if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey || event.button !== 0)
    return;
  event.preventDefault();
  navigate(event.currentTarget.getAttribute('href')!);
}

/** The mock's state lives in the query (#5821), so a capture and a shared link land on the same state. */
export function useQuery(): URLSearchParams {
  const search = useSyncExternalStore(subscribe, () => location.search);
  return new URLSearchParams(search);
}
export function setQuery(changes: Record<string, string | null>) {
  const query = new URLSearchParams(location.search);
  for (const [key, value] of Object.entries(changes))
    if (value === null) query.delete(key);
    else query.set(key, value);
  const search = query.toString();
  history.replaceState(null, '', `${location.pathname}${search ? `?${search}` : ''}`);
  dispatchEvent(new PopStateEvent('popstate'));
}
