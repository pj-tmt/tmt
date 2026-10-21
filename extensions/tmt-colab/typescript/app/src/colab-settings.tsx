import { useEffect, useState } from 'react';
import { BrowserList, BrowserListRow } from '@tmt/browser-ui/react';
import {
  colabSettingsSource,
  type ColabSettingsSource,
  type ColabSettingsUnavailable,
  type ColabSettingsView,
} from './colab-settings-source.js';

/** Where the read this page waits for is requested. */
const READ_REQUEST = 'https://github.com/pj-tmt/tmt/issues/3937#issuecomment-6321040144';
import { text } from './strings.js';

/** Colab's own settings, read-only: each row names the setting, what it does and its value. The
 * shell's header names the page. */
export function ColabSettings({ source = colabSettingsSource }: { source?: ColabSettingsSource }) {
  const [view, setView] = useState<ColabSettingsView | ColabSettingsUnavailable>();
  useEffect(() => {
    let live = true;
    void source.read().then((next) => live && setView(next));
    return () => {
      live = false;
    };
  }, [source]);
  const t = text.colabSettings;
  if (view && 'unavailable' in view)
    return (
      <section className="home colab-settings" aria-label={t.title}>
        <p className="colab-settings-note" role="note">
          {view.unavailable} <a href={READ_REQUEST}>Read request</a>
        </p>
      </section>
    );
  const agents = view
    ? Object.entries(view.wipLimit.agents).map(([agent, limit]) => t.wipAgent(agent, limit))
    : [];
  return (
    <section className="home colab-settings" aria-label={t.title}>
      {view && (
        <>
          <p className="colab-settings-note" role="note">
            {view.sample ? t.sample : t.readOnly}
          </p>
          <BrowserList label={t.title}>
            <BrowserListRow title={t.open} metadata={t.openHelp} state={view.open ? t.on : t.off} />
            <BrowserListRow
              title={t.wip}
              metadata={t.wipHelp}
              state={[String(view.wipLimit.default), ...agents].join(' · ')}
            />
            <BrowserListRow
              title={t.github}
              metadata={t.githubHelp}
              state={view.githubRepo ?? t.notSet}
            />
            <BrowserListRow
              title={t.expiry}
              metadata={t.expiryHelp}
              state={view.pageExpiryDays === null ? t.never : t.days(view.pageExpiryDays)}
            />
          </BrowserList>
        </>
      )}
    </section>
  );
}
