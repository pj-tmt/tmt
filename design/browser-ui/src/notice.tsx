import type { ReactNode } from 'react';
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
}: BrowserNoticeProps) {
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
      <h2 className={c.noticeHeading}>{title}</h2>
      <div className={c.noticeBody}>{body}</div>
      {actions !== undefined && <div className={c.noticeActions}>{actions}</div>}
    </section>
  );
}
