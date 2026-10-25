import { BrowserAction, BrowserDrawer, BusyMark } from '@tmt/browser-ui/react';
import type { MutableRefObject, ReactNode } from 'react';
import { PageSizeNotice, type PageSize } from './page-size.js';
import { SaveNotice } from './save-notice.js';
import { SourceEditor } from './source-editor.js';
import { text } from './strings.js';
import type { PageBinding, PageView } from './transport.js';
import { usePageEditor, type SourceDraft } from './use-page-editor.js';

/**
 * The owner's Source drawer: the page's draft, its Save and the Save's notices. It alone
 * subscribes to the draft, so a keystroke renders this drawer and not the page around it.
 */
export function PageSource({
  draft,
  binding,
  latest,
  size,
  open,
  notice,
  focusFallback,
  archived,
  disabled,
  onDismiss,
  exportPage,
}: {
  draft: SourceDraft;
  binding: PageBinding | undefined;
  /** The newest view the page holds, which a landed Save adopts as the new base. */
  latest: MutableRefObject<PageView>;
  size: PageSize;
  open: boolean;
  notice: ReactNode;
  focusFallback: () => HTMLElement | null;
  archived: boolean;
  /** Writes are held: the connection is not live. */
  disabled: boolean;
  onDismiss: () => void;
  exportPage: () => void;
}) {
  const {
    draft: value,
    changed,
    saving,
    error,
    merged,
    copied,
    sourceArea,
    save,
    copyDraft,
    compactFromRefusal,
    type,
  } = usePageEditor({ draft, binding, latest, size });
  return (
    <BrowserDrawer
      notice={notice}
      focusFallback={focusFallback}
      open={open}
      title={text.source}
      panel="source"
      width="wide"
      headerActions={
        binding &&
        !archived && (
          <BrowserAction
            type="button"
            variant="primary"
            label={text.save}
            visibleLabel={text.sourceSave}
            busy={saving}
            busyMark={<BusyMark />}
            disabled={disabled || !changed}
            onActivate={() => void save()}
          />
        )
      }
      onDismiss={onDismiss}
    >
      <SourceEditor
        changed={changed}
        editor={{
          ref: sourceArea,
          readOnly: archived || !binding || saving || disabled,
          value,
          onChange: (event) => type(event.target.value),
        }}
      >
        {/* Save problems take priority over routine usage in this one notice slot. */}
        {error ? (
          <SaveNotice
            problem={error}
            actions={
              error.full === 'compact' ? (
                <BrowserAction
                  type="button"
                  label={text.pageSize.compact}
                  variant="text"
                  disabled={size.compacting}
                  onActivate={() => void compactFromRefusal()}
                />
              ) : error.full === 'export' ? (
                <BrowserAction
                  type="button"
                  label={text.pageSize.export}
                  variant="text"
                  onActivate={exportPage}
                />
              ) : error.conflict ? (
                <>
                  <BrowserAction
                    type="button"
                    label={text.saveCopyDraft}
                    variant="text"
                    onActivate={(event) => {
                      if (event.isTrusted) copyDraft();
                    }}
                  />
                  {copied && (
                    <span role="status">
                      {copied === 'copied' ? text.saveDraftCopied : text.saveDraftNotCopied}
                    </span>
                  )}
                </>
              ) : undefined
            }
          />
        ) : (
          <PageSizeNotice size={size} exportPage={exportPage} />
        )}
        {merged && !error && (
          <div className="source-notice">
            <p role="status">{text.saveMerged}</p>
          </div>
        )}
      </SourceEditor>
    </BrowserDrawer>
  );
}
