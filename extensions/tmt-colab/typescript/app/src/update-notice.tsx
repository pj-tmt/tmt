import { BrowserAction } from '@tmt/browser-ui/react';
import { useStaleBuild } from './app-build.js';
import { text } from './strings.js';
import './update-notice.css';

/** Non-blocking: it explains a stale tab and offers Reload; it never reloads by itself. */
export function UpdateNotice() {
  if (!useStaleBuild()) return null;
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
