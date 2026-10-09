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

/** Aperture paths and viewBox copied from site/src/home/assets/v9-0.svg. */
function HeaderMark() {
  return (
    <svg className={c.mark} viewBox="0 0 200 200" aria-hidden="true" fill="currentColor">
      {[0, 60, 120, 180, 240, 300].map((angle) => (
        <g key={angle} transform={`rotate(${angle} 100 100)`}>
          <path d="M100 18A82 82 0 0 1 164 49C143 45 117 55 110 77L92 75C85 50 87 31 100 18Z" />
        </g>
      ))}
    </svg>
  );
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
