import type { ElementType, ReactNode } from 'react';
import { browserUiClasses as c } from './static';
import type { BrowserAnnouncement, BrowserNoticeTone } from './static';
export interface BrowserNoticeProps {
  tone: BrowserNoticeTone;
  announcement: BrowserAnnouncement;
  mark: ReactNode;
  stateLabel: string;
  eyebrow?: string;
  title: string;
  body: ReactNode;
  actions?: ReactNode;
  /** `none` renders the title as a styled block, for a sample that must stay out of the page's heading outline. */
  headingLevel?: 'h2' | 'none';
}
export function BrowserNotice({
  tone,
  announcement,
  mark,
  stateLabel,
  eyebrow,
  title,
  body,
  actions,
  headingLevel = 'h2',
}: BrowserNoticeProps) {
  const Title: ElementType = headingLevel === 'none' ? 'div' : 'h2';
  return (
    <section
      className={c.notice}
      data-tone={tone}
      role={announcement === 'none' ? undefined : announcement}
    >
      {eyebrow !== undefined && <div className={c.noticeEyebrow}>{eyebrow}</div>}
      <div className={c.noticeMark}>
        <span aria-hidden="true">{mark}</span>
        <span>{stateLabel}</span>
      </div>
      <Title className={c.noticeHeading}>{title}</Title>
      <div className={c.noticeBody}>{body}</div>
      {actions !== undefined && <div className={c.noticeActions}>{actions}</div>}
    </section>
  );
}
