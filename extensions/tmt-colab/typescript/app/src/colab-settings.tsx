import { useEffect, useState, useSyncExternalStore } from 'react';
import { Alert, AlertDescription, UiTheme } from '@tmt/browser-ui/ui';
import '@tmt/browser-ui/ui.css';
import { getTheme, subscribeTheme } from './theme.js';
import {
  ArrowUpRight,
  ChartNoAxesColumn,
  Clock3,
  Link,
  Terminal,
  TriangleAlert,
} from 'lucide-react';
import {
  colabSettingsReferenceDefaults,
  colabSettingsSource,
  type ColabSettingsSource,
  type ColabSettingsUnavailable,
  type ColabSettingsView,
} from './colab-settings-source.js';
import { CopyCommand } from './copy-command.js';
import { text } from './strings.js';
import './colab-settings.css';

/** The reference schema and a real injected read are distinct; no browser settings write exists. */
export function ColabSettings({ source = colabSettingsSource }: { source?: ColabSettingsSource }) {
  const [view, setView] = useState<ColabSettingsView | ColabSettingsUnavailable>();
  useEffect(() => {
    let live = true;
    setView(undefined);
    void source.read().then((next) => live && setView(next));
    return () => {
      live = false;
    };
  }, [source]);
  const theme = useSyncExternalStore(subscribeTheme, getTheme);
  const t = text.colabSettings;
  const unavailable = view && 'unavailable' in view ? view : undefined;
  const values = view && ('unavailable' in view ? colabSettingsReferenceDefaults : view);
  const reference = values?.sample === true;
  const heading = reference ? t.reference : t.current;
  const agents = values
    ? Object.entries(values.wipLimit.agents).map(([agent, limit]) => t.wipAgent(agent, limit))
    : [];
  const rows = values
    ? [
        { icon: ArrowUpRight, title: t.open, help: t.openHelp, value: values.open ? t.on : t.off },
        {
          icon: ChartNoAxesColumn,
          title: t.wip,
          help: t.wipHelp,
          value: [t.perAgent(values.wipLimit.default), ...agents].join(' · '),
        },
        { icon: Link, title: t.github, help: t.githubHelp, value: values.githubRepo ?? t.notSet },
        {
          icon: Clock3,
          title: t.expiry,
          help: t.expiryHelp,
          value: values.pageExpiryDays === null ? t.never : t.days(values.pageExpiryDays),
        },
      ]
    : [];
  return (
    <section className="home colab-settings" aria-label={t.title}>
      <header className="colab-settings-hero">
        <h1>{t.title}</h1>
        <p>{t.subtitle}</p>
      </header>
      {unavailable && (
        <UiTheme theme={theme}>
          <Alert role="note">
            <TriangleAlert aria-hidden className="colab-settings-warning" />
            <AlertDescription>
              <div className="colab-settings-handoff">
                <strong>{unavailable.unavailable}</strong>
                <p>{t.terminalHelp}</p>
                {unavailable.command && <CopyCommand command={unavailable.command} enabled />}
              </div>
            </AlertDescription>
          </Alert>
        </UiTheme>
      )}
      {values && (
        <section className="colab-settings-card" aria-labelledby="colab-settings-values">
          <div className="colab-settings-card-heading">
            <div>
              <h2 id="colab-settings-values">{heading}</h2>
              <p>{reference ? t.referenceHelp : t.readOnly}</p>
            </div>
            <span className="colab-settings-badge">
              <Terminal aria-hidden />
              {t.cliManaged}
            </span>
          </div>
          <dl className="colab-settings-rows">
            {rows.map(({ icon: Icon, title, help, value }) => (
              <div className="colab-settings-row" key={title}>
                <dt>
                  <span className="colab-settings-icon">
                    <Icon aria-hidden />
                  </span>
                  <span>
                    <strong>{title}</strong>
                    <span className="colab-settings-help">{help}</span>
                  </span>
                </dt>
                <dd>{value}</dd>
              </div>
            ))}
          </dl>
        </section>
      )}
      {values && (
        <p className="colab-settings-footer">
          {heading} · {t.readOnlyLabel}
        </p>
      )}
    </section>
  );
}
