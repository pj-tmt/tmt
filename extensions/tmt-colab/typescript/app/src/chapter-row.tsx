import type { ComponentProps, ReactNode } from 'react';

/** One chapter geometry for Today and owner collections. The caller owns progress and actions. */
export function ChapterRow({
  main,
  state,
  before,
  ...props
}: Omit<ComponentProps<'li'>, 'children'> & {
  main: ReactNode;
  state: ReactNode;
  before?: ReactNode;
}) {
  return (
    <li {...props} className={['today-chapter', props.className].filter(Boolean).join(' ')}>
      {before}
      <article>
        <div className="today-chapter-main">{main}</div>
        <div className="today-chapter-state">{state}</div>
      </article>
    </li>
  );
}
