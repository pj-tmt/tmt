import { createBusyMark } from '../../../../../design/browser-ui/src/static.js';

/** A Remote page's wait for its first read: the busy mark, centred where the read fills. */
export function pageLoading(): HTMLElement {
  const row = document.createElement('p');
  row.className = 'tmt-page-loading';
  row.setAttribute('role', 'status');
  row.setAttribute('aria-label', 'Loading');
  row.append(createBusyMark(document));
  return row;
}
