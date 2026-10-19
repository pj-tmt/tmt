import { BrowserAction, BrowserDialog } from '@tmt/browser-ui/react';
import { text } from './strings.js';
import './project-choice-dialog.css';

const t = text.pageList;

/** A phone's way to pick the project for a bulk add or remove: the bar's menus need room it lacks. */
export function ProjectChoiceDialog({
  mode,
  projects,
  onChoose,
  onNewProject,
  onCancel,
}: {
  /** `undefined` keeps the dialog closed. */
  mode: 'add' | 'remove' | undefined;
  projects: readonly string[];
  onChoose(project: string): void;
  onNewProject(): void;
  onCancel(): void;
}) {
  const title = mode === 'remove' ? t.removeFromProject : t.addToProject;
  return (
    <BrowserDialog open={mode !== undefined} title={title} onDismiss={onCancel}>
      <ul className="project-choice">
        {projects.map((name) => (
          <li key={name}>
            <BrowserAction
              type="button"
              variant="text"
              label={name}
              onActivate={() => onChoose(name)}
            />
          </li>
        ))}
        {mode === 'add' && (
          <li>
            <BrowserAction
              type="button"
              variant="text"
              label={t.newProjectItem}
              onActivate={onNewProject}
            />
          </li>
        )}
      </ul>
    </BrowserDialog>
  );
}
