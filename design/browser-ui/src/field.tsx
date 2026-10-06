import type { ReactNode } from 'react';
import { browserUiClasses as c } from './static';
export interface BrowserFieldControlProps {
  id: string;
  className: string;
  'aria-describedby': string | undefined;
  'aria-invalid': boolean | undefined;
}
export interface BrowserFieldProps {
  controlId: string;
  label: string;
  description?: ReactNode;
  descriptionId?: string;
  error?: ReactNode;
  errorId?: string;
  describedByIds?: readonly string[];
  invalid?: boolean;
  renderControl: (props: BrowserFieldControlProps) => ReactNode;
}
export function BrowserField({
  controlId,
  label,
  description,
  descriptionId,
  error,
  errorId,
  describedByIds = [],
  invalid,
  renderControl,
}: BrowserFieldProps) {
  const hasDescription = description !== undefined && description !== null;
  const hasError = error !== undefined && error !== null;
  const localIds = [hasDescription ? descriptionId : undefined, hasError ? errorId : undefined];
  if (
    !controlId.trim() ||
    /\s/.test(controlId) ||
    (hasDescription && !descriptionId) ||
    (hasError && !errorId)
  )
    throw new Error('Field content requires explicit nonempty IDs');
  const ids = [controlId, ...localIds.filter((id): id is string => id !== undefined)];
  if (ids.some((id) => !id.trim() || /\s/.test(id)) || new Set(ids).size !== ids.length)
    throw new Error('Field IDs must be distinct single IDs');
  const describedBy =
    [
      ...new Set([
        ...describedByIds.flatMap((id) => id.split(/\s+/).filter(Boolean)),
        ...ids.slice(1),
      ]),
    ].join(' ') || undefined;
  return (
    <div className={c.field}>
      <label className={c.fieldLabel} htmlFor={controlId}>
        {label}
      </label>
      {renderControl({
        id: controlId,
        className: c.fieldControl,
        'aria-describedby': describedBy,
        'aria-invalid': invalid,
      })}
      {hasDescription && (
        <div className={c.fieldDescription} id={descriptionId}>
          {description}
        </div>
      )}
      {hasError && (
        <div className={c.fieldError} id={errorId}>
          {error}
        </div>
      )}
    </div>
  );
}
