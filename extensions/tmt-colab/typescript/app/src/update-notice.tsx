import { BrowserAction } from '@tmt/browser-ui/react';
import { useEffect } from 'react';
import { useStaleBuild } from './app-build.js';
import { text } from './strings.js';
import './update-notice.css';

/** Non-blocking: it explains a stale tab and offers Reload; it never reloads by itself. */
export function UpdateNotice() {
  const stale = useStaleBuild();
  // The stylesheet reserves the row's height so content moves down instead of being covered.
  useEffect(() => {
    if (!stale) return;
    document.documentElement.setAttribute('data-colab-update', '');
    return () => document.documentElement.removeAttribute('data-colab-update');
  }, [stale]);
  if (!stale) return null;
  return (
    <div className="update-notice" role="status" data-update-notice>
      <span>{text.updated}</span>
      <BrowserAction
        type="button"
        variant="text"
        label={text.reload}
        onActivate={() => location.reload()}
      />
    </div>
  );
}
