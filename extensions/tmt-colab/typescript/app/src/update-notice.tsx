import { BrowserAction } from '@tmt/browser-ui/react';
import { useEffect } from 'react';
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
  if (!stale) return null;
  if (serve?.installed)
    return (
      <div className="update-notice" role="status" data-update-notice data-serve-older>
        <span>
          {text.serveOlder.lead(serve.installed, serve.running)} {text.serveOlder.restart}{' '}
          <code>{text.serveOlder.command}</code> {text.serveOlder.tail}
        </span>
      </div>
    );
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
