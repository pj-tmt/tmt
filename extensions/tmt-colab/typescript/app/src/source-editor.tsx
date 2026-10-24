import type { ComponentPropsWithRef, ReactNode } from 'react';
import { text } from './strings.js';
import './source-editor.css';

/** Presentation shared by the owner and editor link; each caller retains its draft and save. */
export function SourceEditor({
  changed,
  editor,
  children,
}: {
  changed: boolean;
  editor: ComponentPropsWithRef<'textarea'>;
  children?: ReactNode;
}) {
  return (
    <div className="source">
      <p className="source-explanation">{text.sourceExplanation}</p>
      {children}
      <textarea
        {...editor}
        id="source-edit"
        aria-label={text.sourceEditorLabel}
        spellCheck={false}
      />
      <div className="source-metadata">
        <span data-testid="source-draft-status">
          {changed ? text.sourceDraftChanged : text.sourceDraftUnchanged}
        </span>
        <span>{text.sourceEditorLabel}</span>
      </div>
    </div>
  );
}
