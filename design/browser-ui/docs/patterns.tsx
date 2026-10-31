import type { ReactNode } from 'react';
import {
  Badge,
  Button,
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
  Tabs,
  TabsList,
  TabsTrigger,
} from '../src/ui/index.js';
import { follow } from './router.js';

/** A list row: title, tags, time and an unread state in weight alone. */
export function PageRow({
  title,
  tags,
  updated,
  unread = false,
  state,
}: {
  title: string;
  tags: string[];
  updated: string;
  unread?: boolean;
  state?: 'waiting' | 'blocked';
}) {
  return (
    <a
      className="pattern-row"
      href="/mock/colab/page"
      onClick={follow}
      data-unread={unread || undefined}
    >
      <span className="pattern-row-title">
        {state && (
          <span className="pattern-state" data-tone={state}>
            {state === 'waiting' ? '◆' : '✕'}
          </span>
        )}
        {title}
        {unread && <span className="tmt-ui-visually-hidden"> unread</span>}
      </span>
      <span className="pattern-row-tags">
        {tags.map((tag) => (
          <Badge key={tag} variant="outline">
            {tag}
          </Badge>
        ))}
      </span>
      <span className="pattern-row-time">{updated}</span>
    </a>
  );
}

export const patterns: { slug: string; title: string; lead: string; render(): ReactNode }[] = [
  {
    slug: 'list-row',
    title: 'List row',
    lead: 'One object per row: its title first, then tags and time. Unread is bold, never a colour; a state carries its mark.',
    render: () => (
      <div className="docs-canvas docs-canvas-flush">
        <PageRow title="Release plan" tags={['spec', 'tmt']} updated="2 min ago" unread />
        <PageRow title="Theme B review" tags={['design']} updated="1 h ago" state="waiting" />
        <PageRow title="CI flake triage" tags={['infra']} updated="Yesterday" state="blocked" />
      </div>
    ),
  },
  {
    slug: 'page-header',
    title: 'Page header with tabs',
    lead: 'The title, one line of context, the primary action on the right and the views as line tabs.',
    render: () => (
      <div className="docs-canvas docs-canvas-flush">
        <div className="pattern-header">
          <div>
            <h2 className="pattern-title">Pages</h2>
            <p className="docs-muted">38 pages · 4 need you</p>
          </div>
          <Button type="button">New page</Button>
        </div>
        <Tabs defaultValue="all">
          <TabsList variant="line">
            <TabsTrigger value="all">All</TabsTrigger>
            <TabsTrigger value="needs">Needs you</TabsTrigger>
            <TabsTrigger value="mine">Mine</TabsTrigger>
            <TabsTrigger value="archived">Archived</TabsTrigger>
          </TabsList>
        </Tabs>
      </div>
    ),
  },
  {
    slug: 'empty-state',
    title: 'Empty state',
    lead: 'Say what would be here and give one way to add it.',
    render: () => (
      <div className="docs-canvas">
        <Empty>
          <EmptyHeader>
            <EmptyTitle>No pages in this project</EmptyTitle>
            <EmptyDescription>Move a page here, or start one from a template.</EmptyDescription>
          </EmptyHeader>
          <EmptyContent>
            <Button type="button" variant="outline" size="sm">
              New page
            </Button>
          </EmptyContent>
        </Empty>
      </div>
    ),
  },
  {
    slug: 'status-line',
    title: 'Status line',
    lead: 'States are a mark plus a word in their muted hue. The accent never colours a state.',
    render: () => (
      <div className="docs-canvas">
        <ul className="docs-status">
          <li style={{ color: 'var(--tmt-ui-color-working)' }}>● Working</li>
          <li style={{ color: 'var(--tmt-ui-color-waiting)' }}>◆ Waiting on you</li>
          <li style={{ color: 'var(--tmt-ui-color-review)' }}>◐ In review</li>
          <li style={{ color: 'var(--tmt-ui-color-blocked)' }}>✕ Blocked</li>
        </ul>
      </div>
    ),
  },
  {
    slug: 'planned',
    title: 'Planned parts',
    lead: 'Parts the Colab mock still builds from stand-ins. Each is planned; the rich inputs and the collection list are #5830.',
    render: () => (
      <ul className="docs-planned">
        <li>
          <b>ToggleGroup</b> — a set of pressed buttons, for the list's project filters. Today:
          ghost and secondary Buttons with aria-pressed.
        </li>
        <li>
          <b>Tabs count slot</b> — a muted count inside a tab trigger. Today: plain text in the
          label.
        </li>
        <li>
          <b>Avatar</b> — initials or a picture in the pill radius. Today: initials in a span.
        </li>
        <li>
          <b>List row</b> — the title, tags, time and unread row as one part. Today: the List row
          pattern above.
        </li>
      </ul>
    ),
  },
];
