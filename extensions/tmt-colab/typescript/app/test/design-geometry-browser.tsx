/** Real @tmt/browser-ui busy actions and chips for geometry checks; no product state. */
import '@tmt/browser-ui/static.css';
import '@tmt/browser-ui/ui.css';
import { createRoot } from 'react-dom/client';
import { BrowserAction, BrowserChip, BusyMark } from '@tmt/browser-ui/react';
import { Chip, UiTheme } from '@tmt/browser-ui/ui';

const variants = ['primary', 'text', 'destructive'] as const;
const noop = () => {};

export function mountDesignGeometry() {
  const host = document.createElement('div');
  host.id = 'geometry-fixture';
  document.body.append(host);
  createRoot(host).render(
    <>
      {(['light', 'dark'] as const).map((theme) => (
        <section key={theme} data-theme={theme} data-geometry={theme}>
          {variants.map((variant) => (
            <BrowserAction
              key={variant}
              type="button"
              variant={variant}
              label={`Busy ${variant} ${theme}`}
              busy
              busyMark={<BusyMark />}
              onActivate={noop}
            />
          ))}
          <BrowserChip
            label={`core-ops ${theme}`}
            removeLabel={`Remove ${theme}`}
            onRemove={noop}
          />
          <UiTheme theme={theme}>
            <Chip
              item={{ id: 'ux', label: `ux-4 ${theme}`, removeLabel: `Remove ui ${theme}` }}
              onRemove={noop}
            />
          </UiTheme>
        </section>
      ))}
    </>,
  );
}
