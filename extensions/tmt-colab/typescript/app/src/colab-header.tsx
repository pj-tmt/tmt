import type { ReactNode, Ref } from 'react';
import { text } from './strings.js';
import './colab-header.css';

/** Shared chrome; screens supply content, never header geometry or typography. */
export function ColabHeader({
  title,
  caption,
  actions,
  home,
  headerRef,
  menuOpen,
}: {
  title: string;
  caption?: string;
  actions?: ReactNode;
  home?: (brand: ReactNode) => ReactNode;
  headerRef?: Ref<HTMLElement>;
  menuOpen?: boolean;
}) {
  const brand = (
    <>
      <span className="colab-mark">tmt</span>
      <span className="colab-wordmark">{text.product}</span>
    </>
  );
  return (
    <header className="colab-header" ref={headerRef} data-menu-open={menuOpen}>
      {home ? home(brand) : <span className="colab-brand">{brand}</span>}
      <div className="colab-heading">
        <h1 className="colab-title" title={title}>
          {title}
        </h1>
        {caption !== undefined && (
          <div className="colab-caption" title={caption}>
            {caption}
          </div>
        )}
      </div>
      <div className="colab-actions">{actions}</div>
    </header>
  );
}
