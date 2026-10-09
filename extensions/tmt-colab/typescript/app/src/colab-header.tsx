import type { ReactNode, Ref } from 'react';
import { BrowserHeader } from '@tmt/browser-ui/react';
import { text } from './strings.js';
import './colab-header.css';

/** Colab supplies routing and content to the shared presentation. */
export function ColabHeader({
  title,
  caption,
  actions,
  home,
  headerRef,
  status,
}: {
  title: string;
  caption?: string;
  actions?: ReactNode;
  home?: (brand: ReactNode) => ReactNode;
  headerRef?: Ref<HTMLElement>;
  status?: ReactNode;
}) {
  return (
    <BrowserHeader
      productLabel={text.product}
      title={title}
      caption={caption}
      captionId={caption === undefined ? undefined : 'colab-header-caption'}
      brandLink={home}
      actions={actions ?? <></>}
      status={status}
      headerRef={headerRef}
    />
  );
}
