import { BrowserAction, BrowserDialog } from '@tmt/browser-ui/react';
import { text } from './strings.js';
import type { PageOperation } from './page-operations.js';
import './page-operation-dialog.css';

const t = text.pageList;
/** A confirmation of many pages lists the first few by title and counts the rest. */
const LISTED = 5;

/** Archive and delete name what they change and say what cannot be undone. The host runs the
 * statements and reports the outcome in the list's status line, so the dialog only confirms. */
export function PageOperationDialog({
  operation,
  titles,
  busy,
  onConfirm,
  onCancel,
}: {
  /** `undefined` keeps the dialog closed. */
  operation: PageOperation | undefined;
  /** The pages' titles, in the order they will be changed. */
  titles: readonly string[];
  busy: boolean;
  onConfirm(): void;
  onCancel(): void;
}) {
  const deleting = operation === 'page.delete';
  const many = titles.length > 1;
  const heading = many
    ? deleting
      ? t.confirmDeleteMany(titles.length)
      : t.confirmArchiveMany(titles.length)
    : deleting
      ? t.confirmDeleteTitle(titles[0] ?? '')
      : t.confirmArchiveTitle(titles[0] ?? '');
  const body = many
    ? deleting
      ? t.confirmDeleteManyBody
      : t.confirmArchiveManyBody
    : deleting
      ? t.confirmDelete
      : t.confirmArchive;
  return (
    <BrowserDialog
      open={operation !== undefined}
      title={heading}
      onDismiss={() => {
        if (!busy) onCancel();
      }}
    >
      <div className="page-operation">
        <p>{body}</p>
        {many && (
          <ul className="page-operation-titles">
            {titles.slice(0, LISTED).map((title, index) => (
              <li key={index}>{title}</li>
            ))}
            {titles.length > LISTED && <li>{t.moreTitles(titles.length - LISTED)}</li>}
          </ul>
        )}
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
            label={
              deleting
                ? many
                  ? t.deleteButtonMany
                  : t.deleteButton
                : many
                  ? t.archiveButtonMany
                  : t.archiveButton
            }
            busy={busy}
            onActivate={onConfirm}
          />
        </div>
      </div>
    </BrowserDialog>
  );
}
