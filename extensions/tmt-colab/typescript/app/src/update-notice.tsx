import { BrowserAction } from '@tmt/browser-ui/react';
import { useEffect, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useServeOlder, useStaleBuild } from './app-build.js';
import { text } from './strings.js';
import './update-notice.css';

/**
 * Non-blocking: it explains a stale tab and offers Reload, or a serve older than the installed
 * release and says to restart it (a Reload cannot help). It never reloads or restarts anything.
 */
export function UpdateNotice() {
  const tabStale = useStaleBuild();
  const serve = useServeOlder();
  const stale = tabStale || !!serve;
  // The stylesheet reserves the row's height so content moves down instead of being covered.
  useEffect(() => {
    if (!stale) return;
    document.documentElement.setAttribute('data-colab-update', serve?.installed ? 'serve' : '');
    return () => document.documentElement.removeAttribute('data-colab-update');
  }, [stale, serve]);
  // Inside the TMT web shell the row joins the shell's sticky bar under the header, so it spans
  // Colab's column and moves the page down with the bar.
  const [bar, setBar] = useState<Element | null>(null);
  useEffect(() => {
    if (!stale) return;
    // The shell may commit its bar after the check that found the tab stale.
    const find = () => {
      const found = document.querySelector('.tmt-ui-shell-bar');
      if (found) setBar(found);
      return !!found;
    };
    if (find()) return;
    const watch = new MutationObserver(() => find() && watch.disconnect());
    watch.observe(document.body, { childList: true, subtree: true });
    return () => watch.disconnect();
  }, [stale]);
  if (!stale) return null;
  const place = (row: ReactNode) => (bar ? createPortal(row, bar) : row);
  if (serve?.installed)
    return place(
      <div className="update-notice" role="status" data-update-notice data-serve-older>
        <span>
          {text.serveOlder.lead(serve.installed, serve.running)} {text.serveOlder.restart}{' '}
          <code className="tmt-ui-code">{text.serveOlder.command}</code> {text.serveOlder.tail}
        </span>
      </div>,
    );
  return place(
    <div className="update-notice" role="status" data-update-notice>
      <span>{text.updated}</span>
      <BrowserAction
        type="button"
        variant="text"
        label={text.reload}
        onActivate={() => location.reload()}
      />
    </div>,
  );
}
