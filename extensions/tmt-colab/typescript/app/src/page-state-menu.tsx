import { BrowserMenu } from '@tmt/browser-ui/react';
import type { PageState } from './bootstrap.js';
import { text } from './strings.js';

/** Manual page-state choices shared by the list and the shortcut editor. */
export function PageStateMenu({
  state,
  onChange,
}: {
  state?: PageState;
  onChange(state?: PageState): void;
}) {
  const label = state ? text.workBand.taskState[state] : text.shortcuts.any;
  return (
    <BrowserMenu
      label={text.shortcuts.stateLabel(label)}
      triggerText={text.shortcuts.state(label)}
      items={(['any', 'todo', 'in-progress', 'done'] as const).map((key) => ({
        key,
        label: key === 'any' ? text.shortcuts.anyState : text.workBand.taskState[key],
        role: 'menuitemradio' as const,
        checked: (state ?? 'any') === key,
      }))}
      onSelect={(key) => onChange(key === 'any' ? undefined : key)}
    />
  );
}
