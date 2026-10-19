import { useState } from 'react';
import { BrowserAction, BrowserMenu } from '@tmt/browser-ui/react';
import { ProjectChoiceDialog } from './project-choice-dialog.js';
import { text } from './strings.js';
import { useMedia } from './use-media.js';
import './bulk-bar.css';

const t = text.pageList;

/** The design system's compact width: a phone gets one row and an Actions menu. */
const phoneQuery = '(max-width: 479px)';

/** What the selected pages can have done to them together. It takes the search row's place while a
 * selection exists; the host owns the selection and every command. */
export function BulkBar({
  count,
  projects,
  carried,
  archiving,
  onAdd,
  onRemove,
  onNewProject,
  onArchive,
  onDelete,
  onClear,
}: {
  count: number;
  /** Every project of the space: what the selection can be added to. */
  projects: readonly string[];
  /** The projects at least one selected page is in: what it can be removed from. */
  carried: readonly string[];
  /** Whether archive applies: the Archived view has nothing left to archive. */
  archiving: boolean;
  onAdd(project: string): void;
  onRemove(project: string): void;
  onNewProject(): void;
  onArchive(): void;
  onDelete(): void;
  onClear(): void;
}) {
  const phone = useMedia(phoneQuery);
  const [choosing, setChoosing] = useState<'add' | 'remove'>();
  if (phone)
    return (
      <div className="bulk-bar" role="region" aria-label={t.bulkActions}>
        <span className="bulk-bar-count" role="status">
          {t.selectedShort(count)}
        </span>
        <span className="bulk-bar-actions">
          <BrowserMenu
            label={t.actionsMenu}
            triggerText={t.actionsMenu}
            items={[
              { key: 'add', label: t.addToProject },
              ...(carried.length ? [{ key: 'remove', label: t.removeFromProject }] : []),
              ...(archiving ? [{ key: 'archive', label: t.archiveSelected }] : []),
              { key: 'delete', label: t.deleteSelected, separated: true },
            ]}
            onSelect={(key) => {
              if (key === 'add' || key === 'remove') setChoosing(key);
              else if (key === 'archive') onArchive();
              else onDelete();
            }}
          />
          <BrowserAction
            type="button"
            variant="text"
            label={t.clearSelection}
            visibleLabel={t.clearSelectionPhone}
            onActivate={onClear}
          />
        </span>
        <ProjectChoiceDialog
          mode={choosing}
          projects={choosing === 'remove' ? carried : projects}
          onChoose={(name) => {
            setChoosing(undefined);
            if (choosing === 'remove') onRemove(name);
            else onAdd(name);
          }}
          onNewProject={() => {
            setChoosing(undefined);
            onNewProject();
          }}
          onCancel={() => setChoosing(undefined)}
        />
      </div>
    );
  return (
    <div className="bulk-bar" role="region" aria-label={t.bulkActions}>
      <span className="bulk-bar-count" role="status">
        {t.selectedCount(count)}
      </span>
      <span className="bulk-bar-actions">
        <BrowserMenu
          label={t.addToProject}
          triggerText={t.addToProject}
          items={[
            ...projects.map((name) => ({ key: `project:${name}`, label: name })),
            {
              key: 'new',
              label: t.newProjectItem,
              ...(projects.length ? { separated: true } : {}),
            },
          ]}
          onSelect={(key) => (key === 'new' ? onNewProject() : onAdd(key.slice('project:'.length)))}
        />
        {carried.length > 0 && (
          <BrowserMenu
            label={t.removeFromProject}
            triggerText={t.removeFromProject}
            items={carried.map((name) => ({ key: name, label: name }))}
            onSelect={onRemove}
          />
        )}
        {archiving && (
          <BrowserAction
            type="button"
            variant="text"
            label={t.archiveSelected}
            onActivate={onArchive}
          />
        )}
        <BrowserAction
          type="button"
          variant="destructive"
          label={t.deleteSelected}
          onActivate={onDelete}
        />
        <BrowserAction
          type="button"
          variant="text"
          label={t.clearSelection}
          visibleLabel={t.clearSelectionShort}
          onActivate={onClear}
        />
      </span>
    </div>
  );
}
