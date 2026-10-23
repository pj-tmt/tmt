import { BrowserIconAction } from '@tmt/browser-ui/react';
import { Search } from 'lucide-react';
import { useQuickSwitcher } from './quick-switcher-context.js';
import { usesCommandKey } from './quick-switcher-key.js';
import { text } from './strings.js';

const t = text.quickSwitcher;

export function QuickSwitcherControl() {
  const owner = useQuickSwitcher();
  if (!owner) return null;
  const command = usesCommandKey();
  return (
    <span className="quick-switcher-control">
      <BrowserIconAction
        type="button"
        variant="text"
        label={t.title}
        tooltipLabel={t.shortcut(command)}
        ariaKeyShortcuts={command ? 'Meta+K' : 'Control+K'}
        hasPopup="dialog"
        expanded={owner.open}
        buttonRef={owner.control}
        icon={
          <>
            <Search />
            <kbd>{command ? '⌘K' : 'Ctrl K'}</kbd>
          </>
        }
        onActivate={(event) => {
          if (event.isTrusted) owner.activate();
        }}
      />
    </span>
  );
}
