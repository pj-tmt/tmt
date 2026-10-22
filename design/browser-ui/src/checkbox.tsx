import { useLayoutEffect, useRef, type ComponentPropsWithoutRef, type Ref } from 'react';
import { browserUiClasses as c } from './static';

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
      // The static leaf keeps off the ui layer's helpers: the two refs are set here.
      ref={(node) => {
        own.current = node;
        if (typeof ref === 'function') return ref(node);
        if (ref) ref.current = node;
      }}
      type="checkbox"
      className={c.checkbox}
    />
  );
}
