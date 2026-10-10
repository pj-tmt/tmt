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

/** The TMT mark: one even-odd path, mirrored in site/src/home/assets/v9-0.svg. */
function HeaderMark() {
  return (
    <svg className={c.mark} viewBox="0 0 96 96" aria-hidden="true" fill="currentColor">
      <path
        fillRule="evenodd"
        d="M20 0H76C87.05 0 96 8.95 96 20V76C96 87.05 87.05 96 76 96H20C8.95 96 0 87.05 0 76V20C0 8.95 8.95 0 20 0ZM58.84 33.59C58.79 33.69 58.73 33.8 58.68 33.89C57.54 33.12 56.48 32.2 55.25 31.56C46.86 27.14 37.03 27.23 28.91 32.34C23.71 35.61 21.76 38.08 17.37 42.47C16.62 41.79 13 39.01 13.32 38.11C13.7 37.06 14.61 36.29 15.35 35.45C18.89 31.49 23.35 28.11 28.44 26.41C37.06 23.55 46.41 23.82 54.01 29.23C55.77 30.48 57.23 32.14 58.84 33.59ZM19.87 45.28C23.1 41.07 23.78 39.67 28.6 36.24C39.52 28.46 53 30.15 62.58 39.04C60.76 39.95 62.99 38.95 58.06 36.71C55.69 35.63 52.11 34.82 49.49 34.68C38.21 34.06 31.29 41.34 23.92 48.71C20.8 46.56 22.11 47.74 19.87 45.28ZM26.73 49.96C28.44 48.14 30.07 46.23 31.87 44.49C38.49 38.13 48.26 35.69 56.97 38.89C58.46 39.43 59.77 40.34 61.17 41.07C59.72 41.55 61.43 41.09 58.06 40.29C56.98 40.03 55.89 39.87 54.79 39.83C51.29 39.68 47.84 40.21 44.65 41.69C43.37 42.29 42.08 42.92 40.91 43.72C38.41 45.43 32.68 51.65 28.44 50.89C27.8 50.77 27.3 50.27 26.73 49.96ZM34.84 52.29C41.1 54.64 48.12 54.44 53.85 50.57C57.98 47.8 61.25 42.94 66.16 42.16C66.68 42.08 67.23 42 67.72 42.16C68.93 42.53 70.01 43.2 71.15 43.72C69.91 45.02 68.78 46.45 67.41 47.61C60.76 53.33 52.48 57.88 43.41 56.19C40.15 55.58 37.3 54.45 34.84 52.29ZM44.81 45.59C47.1 40.44 53.13 42.19 52.45 47.61C50.85 51.33 45.61 51.64 44.81 47.46C44.69 46.85 44.81 46.21 44.81 45.59ZM34.21 54.79C35.56 55.56 36.86 56.46 38.27 57.12C48.99 62.18 59.4 57.68 67.88 50.57C69.85 48.93 71.63 47.05 73.49 45.28C76.04 46.41 74.51 45.57 77.7 48.4C68.28 62.53 46.56 68.68 34.21 54.79ZM38.11 60.4C42.39 62.97 42.53 63.47 47.15 64.76C56.3 67.32 66.34 63.41 73.49 57.75C75.88 55.85 78.06 53.69 80.35 51.67C81.23 52.6 82.12 53.54 83 54.48C75.91 65.11 61 72.45 48.4 67.72C46.11 66.87 42.48 65.01 40.44 63.05C39.6 62.23 38.89 61.28 38.11 60.4Z"
      />
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
