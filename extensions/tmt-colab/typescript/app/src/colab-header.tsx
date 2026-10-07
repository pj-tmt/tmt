import type { ReactNode, Ref } from 'react';
import { BrowserHeader } from '@tmt/browser-ui/react';
import { text } from './strings.js';
import './colab-header.css';

/** Colab supplies routing, disclosure and content to the shared presentation. */
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
  return (
    <BrowserHeader
      productLabel={text.product}
      title={title}
      brandLink={home}
      actions={actions ?? <></>}
      headerRef={headerRef}
      disclosure={
        menuOpen === undefined ? undefined : { attribute: 'data-menu-open', open: menuOpen }
      }
    />
  );
}
