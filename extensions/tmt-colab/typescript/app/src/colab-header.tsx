import type { ReactNode, Ref } from 'react';
import { text } from './strings.js';
import './colab-header.css';

/** Shared chrome; screens supply content, never header geometry or typography. */
export function ColabHeader({
  title,
  actions,
  home,
  headerRef,
  menuOpen,
}: {
  title: string;
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
      <h1 className="colab-title" title={title}>
        {title}
      </h1>
      <div className="colab-actions">{actions}</div>
    </header>
  );
}
