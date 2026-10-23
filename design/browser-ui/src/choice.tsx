import { useId, type ComponentPropsWithoutRef, type ReactNode } from 'react';
import { BrowserCheckbox } from './checkbox.js';
import { browserUiClasses as c } from './static.js';

export interface BrowserChoiceGroupProps {
  /** Names the group: a radio group needs it, a check group should have it. */
  legend: string;
  /** `inline` wraps short choices on a line. */
  layout?: 'stack' | 'inline';
  children: ReactNode;
}

/** A fieldset of choice rows; native radios sharing a `name` keep their arrow-key behaviour. */
export function BrowserChoiceGroup({
  legend,
  layout = 'stack',
  children,
}: BrowserChoiceGroupProps) {
  return (
    <fieldset className={c.choices} data-layout={layout}>
      <legend className={c.choicesLegend}>{legend}</legend>
      {children}
    </fieldset>
  );
}

export type BrowserChoiceProps = Omit<
  ComponentPropsWithoutRef<'input'>,
  'type' | 'className' | 'style' | 'children' | 'aria-describedby'
> & {
  type: 'radio' | 'checkbox';
  label: string;
  /** A sublabel under the label, read with it. */
  description?: string;
  /** A short key or note at the end of the row, such as the digit that picks it; not announced. */
  hint?: string;
};

/** A radio or check row: the control, its label, an optional sublabel and hint; the row is the target. */
export function BrowserChoice({
  type,
  label,
  description,
  hint,
  ...attributes
}: BrowserChoiceProps) {
  const descriptionId = useId();
  const described = description === undefined ? undefined : descriptionId;
  // The hint sits inside the label, where a tool that reads the label's text would add the key to
  // the control's name: a row with a hint names its control by the label text itself.
  const name = attributes['aria-label'] ?? (hint === undefined ? undefined : label);
  return (
    <label className={c.choice}>
      {type === 'radio' ? (
        <input
          {...attributes}
          type="radio"
          className={c.radio}
          aria-label={name}
          aria-describedby={described}
        />
      ) : (
        <BrowserCheckbox {...attributes} aria-label={name} aria-describedby={described} />
      )}
      <span className={c.choiceText}>
        <span>{label}</span>
        {description !== undefined && (
          <span className={c.choiceDescription} id={descriptionId}>
            {description}
          </span>
        )}
      </span>
      {hint !== undefined && (
        <kbd className={c.choiceHint} aria-hidden="true">
          {hint}
        </kbd>
      )}
    </label>
  );
}
