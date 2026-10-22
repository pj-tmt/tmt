import { useLayoutEffect, useRef, useState } from 'react';
import { BrowserTabs } from '../src/react.js';
import { THEMES, applyTheme, type ThemeChoice } from '../src/theme.js';
import { Pair } from './kit.js';
import './theme-cell.css';

const labels: Record<ThemeChoice, string> = { system: 'System', light: 'Light', dark: 'Dark' };

/** A sample surface under one theme choice, set the way `applyTheme` sets the page root. */
function Preview({ choice }: { choice: ThemeChoice }) {
  const root = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => applyTheme(choice, root.current!), [choice]);
  return (
    <div ref={root} className="gallery-theme-preview">
      <strong>{labels[choice]}</strong>
      <span>
        {choice === 'system'
          ? 'No data-theme: the tokens follow this device.'
          : `data-theme="${choice}" pins the ${choice} roles.`}
      </span>
    </div>
  );
}

/** Gallery-only: the per-browser theme choice (src/theme.ts) as the settings page offers it. */
export function ThemeCell() {
  const [selected, setSelected] = useState<ThemeChoice>('system');
  const tabs = THEMES.map((id) => ({
    id,
    tabId: `theme-${id}-tab`,
    panelId: `theme-${id}-panel`,
    label: labels[id],
  }));
  return (
    <Pair
      title="Theme choice (applyTheme, setStoredTheme)"
      note="System removes data-theme from the root; Light and Dark set it. The page stores the choice in this browser."
    >
      {() => (
        <BrowserTabs
          label="Theme"
          tabs={tabs}
          selected={selected}
          onSelect={(id) => setSelected(id as ThemeChoice)}
        >
          {(tab) => <Preview choice={tab.id as ThemeChoice} />}
        </BrowserTabs>
      )}
    </Pair>
  );
}
