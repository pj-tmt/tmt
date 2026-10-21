import { useLayoutEffect, useRef, type ComponentPropsWithoutRef, type Ref } from 'react';
import { browserUiClasses as c } from './static';
import { assignRefs } from './ui/utils.js';

export interface BrowserCheckboxProps extends Omit<
  ComponentPropsWithoutRef<'input'>,
  'type' | 'className' | 'style' | 'children'
> {
  /** The mixed state, drawn as a short bar; `checked` still decides the submitted value. */
  indeterminate?: boolean;
  ref?: Ref<HTMLInputElement>;
}

/**
 * A real `<input type="checkbox">` with the shared drawing: keyboard, labels, forms and
 * `checked`/`defaultChecked` behave natively. Name it with a `<label>` or `aria-label`.
 */
export function BrowserCheckbox({
  indeterminate = false,
  ref,
  ...attributes
}: BrowserCheckboxProps) {
  const own = useRef<HTMLInputElement | null>(null);
  useLayoutEffect(() => {
    own.current!.indeterminate = indeterminate;
  }, [indeterminate]);
  return (
    <input
      {...attributes}
      ref={(node) => assignRefs(node, own, ref)}
      type="checkbox"
      className={c.checkbox}
    />
  );
}
