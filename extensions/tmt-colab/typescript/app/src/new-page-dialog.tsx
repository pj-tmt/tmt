import { BrowserDialog } from '@tmt/browser-ui/react';
import { CopyCommand } from './copy-command.js';
import { text } from './strings.js';
import './page-operation-dialog.css';

const t = text.pageList;

/** The browser does not create pages yet: New page gives the command that does, on the machine. */
export function NewPageDialog({ open, onClose }: { open: boolean; onClose(): void }) {
  return (
    <BrowserDialog open={open} title={t.newPageTitle} onDismiss={onClose}>
      <div className="page-operation">
        <p>{t.newPageBody}</p>
        <CopyCommand command={t.newPageCommand} enabled />
      </div>
    </BrowserDialog>
  );
}
