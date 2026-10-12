import { BrowserMenu, type BrowserMenuProps } from '@tmt/browser-ui/react';
import { useNotificationModalPresence } from './reply-notification-context.js';

/** Menu visibility suspends page notifications; the shared Menu still owns commands and focus. */
export function PageMenu<Key extends string>({ onOpenChange, ...props }: BrowserMenuProps<Key>) {
  const report = useNotificationModalPresence();
  return (
    <BrowserMenu
      {...props}
      onOpenChange={(open) => {
        report(open);
        onOpenChange?.(open);
      }}
    />
  );
}
