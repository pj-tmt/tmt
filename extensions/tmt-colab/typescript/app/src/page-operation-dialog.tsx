import { BrowserAction, BrowserDialog } from '@tmt/browser-ui/react';
import { text } from './strings.js';
import type { PageOperation } from './page-operations.js';
import './page-operation-dialog.css';

const t = text.pageList;
/** A confirmation of many pages lists the first few by title and counts the rest. */
const LISTED = 5;

/** Lifecycle actions name what they change and whether they can be undone. The host runs the
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
  const copy =
    operation === 'page.unarchive'
      ? {
          title: t.confirmRestoreTitle,
          many: t.confirmRestoreMany,
          body: t.confirmRestore,
          manyBody: t.confirmRestoreManyBody,
          button: t.restoreButton,
          manyButton: t.restoreButtonMany,
        }
      : deleting
        ? {
            title: t.confirmDeleteTitle,
            many: t.confirmDeleteMany,
            body: t.confirmDelete,
            manyBody: t.confirmDeleteManyBody,
            button: t.deleteButton,
            manyButton: t.deleteButtonMany,
          }
        : {
            title: t.confirmArchiveTitle,
            many: t.confirmArchiveMany,
            body: t.confirmArchive,
            manyBody: t.confirmArchiveManyBody,
            button: t.archiveButton,
            manyButton: t.archiveButtonMany,
          };
  const heading = many ? copy.many(titles.length) : copy.title(titles[0] ?? '');
  const body = many ? copy.manyBody : copy.body;
  return (
    <BrowserDialog
      open={operation !== undefined}
      title={heading}
      onDismiss={() => {
        if (!busy) onCancel();
      }}
      actions={
        <>
          <BrowserAction
            type="button"
            variant="secondary"
            label={t.cancel}
            disabled={busy}
            onActivate={onCancel}
          />
          <BrowserAction
            type="button"
            variant={deleting ? 'destructive' : 'primary'}
            label={many ? copy.manyButton : copy.button}
            busy={busy}
            onActivate={onConfirm}
          />
        </>
      }
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
      </div>
    </BrowserDialog>
  );
}
