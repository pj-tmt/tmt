import type { ReactNode, Ref } from 'react';
import { browserUiClasses as c } from './static';
export interface BrowserHeaderProps {
  productLabel: string;
  title: string;
  caption?: string;
  captionId?: string;
  brandLink?: (brand: ReactNode) => ReactNode;
  status?: ReactNode;
  actions?: ReactNode;
  headerRef?: Ref<HTMLElement>;
  disclosure?: { attribute: `data-${string}`; open: boolean };
}
export function BrowserHeader({
  productLabel,
  title,
  caption,
  captionId,
  brandLink,
  status,
  actions,
  headerRef,
  disclosure,
}: BrowserHeaderProps) {
  if (caption !== undefined && !captionId)
    throw new Error('Caption requires an explicit association ID');
  const brand = (
    <>
      <span className={c.mark} aria-hidden="true">
        ▚
      </span>
      <span className={c.wordmark}>{productLabel}</span>
    </>
  );
  return (
    <header
      className={c.header}
      ref={headerRef}
      {...(disclosure ? { [disclosure.attribute]: disclosure.open } : {})}
    >
      <div className={c.brand}>{brandLink ? brandLink(brand) : brand}</div>
      <div className={c.heading}>
        <h1
          className={c.title}
          title={title}
          aria-describedby={caption !== undefined ? captionId : undefined}
        >
          {title}
        </h1>
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
