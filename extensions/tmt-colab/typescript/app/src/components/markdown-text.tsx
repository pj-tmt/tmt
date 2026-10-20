import { useRouter } from '@tanstack/react-router';
import { Fragment, useMemo, type ReactNode } from 'react';
import { parseMarkdown, type Block, type Inline } from '../markdown.js';
import { shortLinkRoute } from '../page-link-request.js';
import { MessageText } from './message-text.js';
import './markdown-text.css';

/** A comment or Chat body as the safe Markdown subset (#4112). It builds React elements from the
 * parsed tree, so an author's text is never markup; mentions still become chips in the text. */
export function MarkdownText({
  value,
  names = [],
  className = '',
  origin,
  ...attributes
}: {
  value: string;
  names?: readonly string[];
  className?: string;
  /** The app's origin, for same-space links; the document's own unless a test supplies one. */
  origin?: string;
  'data-testid'?: string;
  'data-empty'?: boolean;
}) {
  const tree = useMemo(() => parseMarkdown(value, origin ?? location.origin), [value, origin]);
  return (
    <div {...attributes} className={`markdown-body ${className}`}>
      {blocksOf(tree, names)}
    </div>
  );
}

// Blank lines between blocks stay in the text (hidden by the layout), so selecting and copying a body
// keeps its paragraphs apart.
function blocksOf(blocks: readonly Block[], names: readonly string[]): ReactNode {
  return blocks.map((block, index) => (
    <Fragment key={index}>
      {index > 0 && '\n\n'}
      {blockOf(block, names)}
    </Fragment>
  ));
}
function blockOf(block: Block, names: readonly string[]): ReactNode {
  switch (block.kind) {
    case 'paragraph':
      return <p>{inlines(block.children, names)}</p>;
    case 'code':
      return (
        <pre className="markdown-code">
          <code>{block.value}</code>
        </pre>
      );
    case 'quote':
      return <blockquote>{blocksOf(block.children, names)}</blockquote>;
    case 'list': {
      const items = block.items.map((item, index) => <li key={index}>{blocksOf(item, names)}</li>);
      return block.ordered ? <ol start={block.start}>{items}</ol> : <ul>{items}</ul>;
    }
  }
}
function inlines(nodes: readonly Inline[], names: readonly string[]): ReactNode {
  return nodes.map((node, index) => {
    switch (node.kind) {
      case 'text':
        return <MessageText key={index} value={node.value} names={names} />;
      case 'code':
        return <code key={index}>{node.value}</code>;
      case 'strong':
        return <strong key={index}>{inlines(node.children, names)}</strong>;
      case 'emphasis':
        return <em key={index}>{inlines(node.children, names)}</em>;
      case 'link':
        return (
          <PageLink key={index} href={node.href}>
            {inlines(node.children, names)}
          </PageLink>
        );
    }
  });
}

/** A link to a page of this space. A plain click opens it in the app, like a page link in the page
 * (#4091); a modified or middle click stays the browser's own. */
function PageLink({ href, children }: { href: string; children: ReactNode }) {
  const router = useRouter({ warn: false });
  return (
    <a
      className="markdown-link"
      href={href}
      onClick={(event) => {
        if (
          !router ||
          !event.nativeEvent.isTrusted ||
          event.defaultPrevented ||
          event.button !== 0 ||
          event.metaKey ||
          event.ctrlKey ||
          event.shiftKey ||
          event.altKey
        )
          return;
        const url = new URL(href, location.href);
        const link = shortLinkRoute(url.pathname, url.hash);
        if (!link || link === 'refused') return;
        event.preventDefault();
        void router.navigate({
          to: link.route,
          search: link.anchor ? { anchor: link.anchor } : {},
        });
      }}
    >
      {children}
    </a>
  );
}
