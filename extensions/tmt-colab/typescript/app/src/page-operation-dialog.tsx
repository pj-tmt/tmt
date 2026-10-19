import { BrowserAction, BrowserDialog } from '@tmt/browser-ui/react';
import { text } from './strings.js';
import type { PageOperation } from './page-operations.js';
import './page-operation-dialog.css';

const t = text.pageList;

/** Archive and delete name the page and say what cannot be undone. The host runs the statement and
 * reports its outcome in the list's status line, so the dialog only confirms. */
export function PageOperationDialog({
  operation,
  title,
  busy,
  onConfirm,
  onCancel,
}: {
  /** `undefined` keeps the dialog closed. */
  operation: PageOperation | undefined;
  title: string;
  busy: boolean;
  onConfirm(): void;
  onCancel(): void;
}) {
  const deleting = operation === 'page.delete';
  return (
    <BrowserDialog
      open={operation !== undefined}
      title={deleting ? t.confirmDeleteTitle(title) : t.confirmArchiveTitle(title)}
      onDismiss={() => {
        if (!busy) onCancel();
      }}
    >
      <div className="page-operation">
        <p>{deleting ? t.confirmDelete : t.confirmArchive}</p>
        <div className="page-operation-actions">
          <BrowserAction
            type="button"
            variant="text"
            label={t.cancel}
            disabled={busy}
            onActivate={onCancel}
          />
          <BrowserAction
            type="button"
            variant={deleting ? 'destructive' : 'primary'}
            label={deleting ? t.deleteButton : t.archiveButton}
            busy={busy}
            onActivate={onConfirm}
          />
        </div>
      </div>
    </BrowserDialog>
  );
}
