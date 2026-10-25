import type { ElementType, ReactNode, Ref } from 'react';
import { browserUiClasses as c } from './static';
import { BrandMark } from './brand-mark.js';
export interface BrowserHeaderProps {
  productLabel: string;
  title: string;
  caption?: string;
  captionId?: string;
  brandLink?: (brand: ReactNode) => ReactNode;
  leading?: ReactNode;
  headingPrefix?: ReactNode;
  headingSuffix?: ReactNode;
  /** `none` renders the title as a styled block, for a sample that must stay out of the page's heading outline. */
  headingLevel?: 'h1' | 'none';
  status?: ReactNode;
  actions?: ReactNode;
  headerRef?: Ref<HTMLElement>;
  disclosure?: { attribute: `data-${string}`; open: boolean };
}

/** The TMT 04D-1 mark in its small form; `generated/brand-mark-static.svg` is its static-host copy. */
function HeaderMark() {
  return <BrandMark className={c.mark} size={24} />;
}

export function BrowserHeader({
  productLabel,
  title,
  caption,
  captionId,
  brandLink,
  leading,
  headingPrefix,
  headingSuffix,
  headingLevel = 'h1',
  status,
  actions,
  headerRef,
  disclosure,
}: BrowserHeaderProps) {
  const Title: ElementType = headingLevel === 'none' ? 'div' : 'h1';
  if (caption !== undefined && !captionId)
    throw new Error('Caption requires an explicit association ID');
  const brand = (
    <>
      <HeaderMark />
      <span className={c.wordmark}>{productLabel}</span>
    </>
  );
  return (
    <header
      className={c.header}
      ref={headerRef}
      {...(disclosure ? { [disclosure.attribute]: disclosure.open } : {})}
    >
      {leading}
      <div className={c.brand}>{brandLink ? brandLink(brand) : brand}</div>
      <div className={c.heading}>
        {headingPrefix}
        <Title
          className={c.title}
          title={title}
          aria-describedby={caption !== undefined ? captionId : undefined}
        >
          {title}
        </Title>
        {headingSuffix !== undefined && <div className={c.headingSuffix}>{headingSuffix}</div>}
        {caption !== undefined && (
          <div className={c.caption} id={captionId} title={caption}>
            {caption}
          </div>
        )}
      </div>
      {status !== undefined && <div className={c.status}>{status}</div>}
      {actions !== undefined && <div className={c.actions}>{actions}</div>}
    </header>
  );
}
