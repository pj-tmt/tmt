import { useState, type ReactNode } from 'react';
import {
  Archive,
  Bookmark,
  ChevronDown,
  ExternalLink,
  FileText,
  ListFilter,
  Lock,
  Pin,
  Plus,
  Search,
  SlidersHorizontal,
  X,
} from 'lucide-react';
import { BrowserTopBar, useTopBarReady } from '../src/react.js';
import {
  Alert,
  AlertDescription,
  AlertTitle,
  Badge,
  Button,
  Checkbox,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  Surface,
  Tabs,
  TabsList,
  TabsTrigger,
  TooltipProvider,
} from '../src/ui/index.js';
import { StatePanel, type StateControl } from './mock-reader.js';
import { follow, setQuery, useQuery } from './router.js';

/*
 * The Colab page list, reconstructed (#5821, GUI lead notes 10 and 11): today's list on main
 * (home.tsx, page-list-items.tsx, page-list-search.tsx: hero actions, pinned pages, stage tabs with
 * counts, the search with its condition chip, Filter and sort, the six-column table) plus Night's
 * #5818 control row and sticky row and #5814 Needs-you cleanup, in palette B and ui parts.
 */

type Stage = 'discussion' | 'in-progress' | 'done' | 'archived';
interface Row {
  title: string;
  projects: string[];
  stage: Stage;
  needs?: number;
  work?: string;
  workers?: string[];
  updated: string;
  author: string;
  access: 'Shared' | 'Private';
  pinned?: boolean;
  unread?: boolean;
}
const rows: Row[] = [
  {
    title: 'Theme B review',
    projects: ['design', 'spec'],
    stage: 'in-progress',
    needs: 2,
    work: '3 of 5 done',
    workers: ['sample-2'],
    updated: '2 min ago',
    author: 'sample-2',
    access: 'Shared',
    pinned: true,
    unread: true,
  },
  {
    title: 'Release plan',
    projects: ['release-0.9'],
    stage: 'in-progress',
    work: '2 of 3 done',
    workers: ['sample-1', 'sample-lead'],
    updated: '18 min ago',
    author: 'sample-lead',
    access: 'Shared',
    pinned: true,
  },
  {
    title: 'Night execution plan',
    projects: ['night-shift'],
    stage: 'discussion',
    needs: 1,
    updated: '40 min ago',
    author: 'sample-lead',
    access: 'Shared',
    unread: true,
  },
  {
    title: 'Rollout checklist',
    projects: ['release-0.9', 'ops'],
    stage: 'in-progress',
    work: '0 of 2 done',
    updated: '1 h ago',
    author: 'sample-1',
    access: 'Shared',
  },
  {
    title: 'CI flake triage',
    projects: ['infra'],
    stage: 'discussion',
    updated: '3 h ago',
    author: 'sample-2',
    access: 'Private',
  },
  {
    title: 'Remote pairing walkthrough',
    projects: ['remote-launch'],
    stage: 'done',
    work: '4 of 4 done',
    updated: 'Yesterday',
    author: 'sample-1',
    access: 'Shared',
    pinned: true,
  },
  {
    title: 'Digest TUI alignment',
    projects: ['spec'],
    stage: 'discussion',
    needs: 1,
    updated: 'Yesterday',
    author: 'sample-lead',
    access: 'Shared',
  },
  {
    title: 'Token tiers',
    projects: ['design'],
    stage: 'done',
    updated: 'Mon',
    author: 'sample-2',
    access: 'Shared',
  },
  {
    title: 'Brand sheet',
    projects: ['brand'],
    stage: 'done',
    updated: '27 Oct',
    author: 'sample-lead',
    access: 'Private',
  },
  {
    title: 'Weekly plan, week 44',
    projects: ['pm'],
    stage: 'discussion',
    updated: '26 Oct',
    author: 'sample-lead',
    access: 'Shared',
  },
];
const stageName: Record<Stage, string> = {
  discussion: 'Discussion',
  'in-progress': 'In progress',
  done: 'Done',
  archived: 'Archived',
};
/** The project chip's hue: one of the data colours by the name's hash, as projectTone does. */
const projectHue = (name: string) =>
  `var(--tmt-ui-color-series-${([...name].reduce((sum, c) => sum + c.charCodeAt(0), 0) % 6) + 1})`;

function ProjectChip({ name }: { name: string }) {
  return (
    <Badge variant="outline" className="list-project">
      <span
        className="list-project-dot"
        style={{ background: projectHue(name) }}
        aria-hidden="true"
      />
      {name}
    </Badge>
  );
}
function StageChip({ stage }: { stage: Stage }) {
  return (
    <span className="list-stage" data-stage={stage}>
      {stageName[stage]}
    </span>
  );
}

const needsItems = [
  ['Theme B review', 'Answer: Which radius ships with B?'],
  ['Theme B review', 'Reply in comment 2'],
  ['Night execution plan', 'Pick the sidebar proposal'],
  ['Digest TUI alignment', 'Review the column widths'],
];

/** The top bar: All pages, the Needs-you menu (#5814: unresolvable items hidden, Clear all asks). */
function ListBar({ needs }: { needs: boolean }) {
  return useTopBarReady() ? (
    <BrowserTopBar
      title={
        <h1>
          <span className="tmt-ui-topbar-title-prefix">Colab / </span>
          <span className="tmt-ui-topbar-title-current">All pages</span>
        </h1>
      }
      actions={
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="list-needs"
              aria-label={needs ? 'Needs you · 4' : 'Needs you · none'}
            >
              {needs && <span className="list-needs-dot" aria-hidden="true" />}
              <span className="list-needs-label">Needs you</span>
              <span className="list-count">{needs ? 4 : 0}</span>
              <ChevronDown aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="list-needs-menu">
            {needs ? (
              <>
                {needsItems.map(([page, what]) => (
                  <DropdownMenuItem key={what} className="list-needs-item">
                    <span className="list-needs-dot" aria-hidden="true" />
                    <span>
                      <b>{page}</b>
                      <span className="list-quiet">{what}</span>
                    </span>
                  </DropdownMenuItem>
                ))}
                <DropdownMenuSeparator />
                <DropdownMenuItem onSelect={() => setQuery({ clear: '1' })}>
                  Clear all…
                </DropdownMenuItem>
              </>
            ) : (
              <DropdownMenuLabel className="list-quiet">Nothing needs you.</DropdownMenuLabel>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      }
    />
  ) : null;
}

function Controls({ stuck, filtered }: { stuck: boolean; filtered: boolean }) {
  return (
    <div className="list-controls" data-stuck={stuck || undefined}>
      <label className="list-search">
        <Search aria-hidden="true" />
        {filtered && (
          <Badge variant="secondary" className="list-condition">
            project: design
            <button
              type="button"
              aria-label="Remove project: design"
              onClick={() => setQuery({ filter: null })}
            >
              <X aria-hidden="true" />
            </button>
          </Badge>
        )}
        <input placeholder="Search pages…" aria-label="Search pages" />
      </label>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button type="button" variant="outline" className="list-control" aria-label="Filter">
            <ListFilter aria-hidden="true" />
            <span className="list-wide">Filter</span>
            <ChevronDown className="list-wide" aria-hidden="true" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuLabel>Project</DropdownMenuLabel>
          {['design', 'spec', 'release-0.9', 'night-shift'].map((name) => (
            <DropdownMenuCheckboxItem
              key={name}
              checked={filtered && name === 'design'}
              onSelect={() => setQuery({ filter: filtered ? null : '1' })}
            >
              {name}
            </DropdownMenuCheckboxItem>
          ))}
          <DropdownMenuSeparator />
          <DropdownMenuCheckboxItem>Needs me</DropdownMenuCheckboxItem>
          <DropdownMenuCheckboxItem>Include archived</DropdownMenuCheckboxItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <span className="list-wide">
        <Select defaultValue="updated">
          <SelectTrigger aria-label="Sort" className="list-control">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="updated">Recently updated</SelectItem>
            <SelectItem value="title">Title</SelectItem>
          </SelectContent>
        </Select>
      </span>
      <Button
        type="button"
        variant="outline"
        size="icon"
        className="list-control list-phone"
        aria-label="Sort"
      >
        <SlidersHorizontal aria-hidden="true" />
      </Button>
      {stuck && (
        <Button type="button" className="list-control">
          <Plus aria-hidden="true" />
          <span className="list-wide">New page</span>
        </Button>
      )}
    </div>
  );
}

function Table({ shown }: { shown: Row[] }) {
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  return (
    <div className="list-scroll">
      <table className="list-table">
        <thead>
          <tr>
            <th className="list-select">
              <Checkbox
                aria-label="Select all matching pages"
                checked={selected.size === shown.length && shown.length > 0}
                onCheckedChange={(on) =>
                  setSelected(on ? new Set(shown.map((row) => row.title)) : new Set())
                }
              />
            </th>
            <th>Title</th>
            <th className="list-col-project">Project</th>
            <th>Stage</th>
            <th className="list-col-updated">Updated</th>
            <th className="list-col-author">Author</th>
            <th className="list-col-access">Access</th>
            <th aria-label="Actions" />
          </tr>
        </thead>
        <tbody>
          {shown.map((row) => (
            <tr
              key={row.title}
              data-selected={selected.has(row.title) || undefined}
              data-unread={row.unread || undefined}
            >
              <td className="list-select">
                <Checkbox
                  aria-label={`Select ${row.title}`}
                  checked={selected.has(row.title)}
                  onCheckedChange={(on) =>
                    setSelected((now) => {
                      const next = new Set(now);
                      if (on) next.add(row.title);
                      else next.delete(row.title);
                      return next;
                    })
                  }
                />
              </td>
              <td>
                <div className="list-title">
                  {row.pinned ? (
                    <Pin className="list-icon" aria-hidden="true" />
                  ) : (
                    <FileText className="list-icon" aria-hidden="true" />
                  )}
                  <a href="/mock/colab/page" onClick={follow}>
                    {row.title}
                  </a>
                </div>
                {row.work && (
                  <span className="list-work">
                    {row.work}
                    {row.workers?.map((who) => (
                      <Badge key={who} variant="secondary">
                        @{who}
                      </Badge>
                    ))}
                  </span>
                )}
                <span className="list-meta-phone">
                  {row.projects.join(' · ')} · {row.updated}
                </span>
              </td>
              <td className="list-col-project">
                <div className="list-projects">
                  {row.projects.map((name) => (
                    <ProjectChip key={name} name={name} />
                  ))}
                </div>
              </td>
              <td>
                {row.needs ? (
                  <span className="list-waiting">
                    <span className="list-needs-dot" aria-hidden="true" /> Needs you · {row.needs}
                  </span>
                ) : (
                  <StageChip stage={row.stage} />
                )}
              </td>
              <td className="list-col-updated list-quiet">{row.updated}</td>
              <td className="list-col-author list-quiet">{row.author}</td>
              <td className="list-col-access list-quiet">
                {row.access === 'Private' && <Lock className="list-lock" aria-hidden="true" />}
                {row.access}
              </td>
              <td className="list-actions">
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  aria-label={row.pinned ? `Unpin ${row.title}` : `Pin ${row.title}`}
                >
                  <Pin aria-hidden="true" />
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  aria-label={`Archive ${row.title}`}
                >
                  <Archive aria-hidden="true" />
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  aria-label={`Open ${row.title} in a new tab`}
                >
                  <ExternalLink aria-hidden="true" />
                </Button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

const views: [string, string, number][] = [
  ['all', 'All', 38],
  ['needs', 'Needs you', 4],
  ['discussion', 'Discussion', 12],
  ['in-progress', 'In progress', 9],
  ['done', 'Done', 13],
  ['archived', 'Archived', 6],
];

export const listControls: StateControl[] = [
  {
    key: 'data',
    label: 'Data',
    options: [
      ['', 'Pages'],
      ['loading', 'Loading'],
      ['empty', 'Empty'],
      ['error', 'Error'],
    ],
  },
  {
    key: 'filter',
    label: 'Filter',
    options: [
      ['', 'None'],
      ['1', 'project: design'],
      ['none', 'Filtered to nothing'],
    ],
  },
  {
    key: 'sticky',
    label: 'Control row',
    options: [
      ['', 'In place'],
      ['1', 'Stuck'],
    ],
  },
  {
    key: 'needs',
    label: 'Needs you',
    options: [
      ['', 'Present'],
      ['cleared', 'Cleared'],
    ],
  },
  {
    key: 'clear',
    label: 'Clear all dialog',
    options: [
      ['', 'Closed'],
      ['1', 'Open'],
    ],
  },
];

const listMissing = [
  'Tabs count slot: a muted count after the label (here a span in the trigger).',
  'Search field with leading icon and condition chips inside (here a label around an input).',
  'Stage chip: dot and word in the stage hue (here a styled span).',
  'Project chip with its hue dot: an outline Badge plus a dot.',
  'Data table: header, selection column, row hover actions, column hiding at the compact width (here a table with classes).',
  'Pinned page card: a sheet Surface with icon, title and meta.',
  'Needs-you dot: the 8px waiting dot shared with the Chat unread dot.',
];

export function ListScreen({
  globals,
  controls = listControls,
}: {
  globals: ReactNode;
  controls?: StateControl[];
}) {
  const query = useQuery();
  const data = query.get('data');
  const filter = query.get('filter');
  const needs = query.get('needs') !== 'cleared';
  const [view, setView] = useState('all');
  const shown =
    filter === 'none'
      ? []
      : rows.filter(
          (row) =>
            (filter !== '1' || row.projects.includes('design')) &&
            (view === 'all' || (view === 'needs' ? row.needs : row.stage === view)),
        );
  return (
    <TooltipProvider>
      <ListBar needs={needs} />
      <div className="list-page">
        <header className="list-hero">
          <Button type="button" variant="outline">
            <Bookmark aria-hidden="true" /> Save as shortcut
          </Button>
          <Button type="button">
            <Plus aria-hidden="true" /> New page
          </Button>
        </header>
        {data !== 'empty' && (
          <ul className="list-pins" aria-label="Pinned pages">
            {rows
              .filter((row) => row.pinned)
              .map((row) => (
                <li key={row.title}>
                  <Surface level="sheet" className="list-pin">
                    <FileText className="list-pin-icon" aria-hidden="true" />
                    <div>
                      <a href="/mock/colab/page" onClick={follow}>
                        {row.title}
                      </a>
                      <div className="list-pin-meta">
                        {row.projects.map((name) => (
                          <ProjectChip key={name} name={name} />
                        ))}
                        <span className="list-quiet">{stageName[row.stage]}</span>
                      </div>
                    </div>
                    <Pin className="list-pin-mark" aria-hidden="true" />
                  </Surface>
                </li>
              ))}
          </ul>
        )}
        <Tabs value={view} onValueChange={setView}>
          <TabsList variant="line" className="list-tabs" aria-label="Page views">
            {views.map(([id, label, count]) => (
              <TabsTrigger key={id} value={id}>
                {label} <span className="list-count">{id === 'needs' && !needs ? 0 : count}</span>
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
        <Controls
          stuck={query.get('sticky') === '1'}
          filtered={filter === '1' || filter === 'none'}
        />
        {data === 'loading' ? (
          <div className="list-loading" aria-label="Loading pages">
            {Array.from({ length: 6 }, (_, index) => (
              <Skeleton key={index} className="list-skeleton" />
            ))}
          </div>
        ) : data === 'error' ? (
          <Alert>
            <AlertTitle>Could not load pages</AlertTitle>
            <AlertDescription>
              The connection to this machine dropped. Pages come back when it reconnects.
              <Button type="button" variant="outline" size="sm" className="list-retry">
                Try again
              </Button>
            </AlertDescription>
          </Alert>
        ) : data === 'empty' ? (
          <Empty>
            <EmptyHeader>
              <EmptyTitle>No pages yet</EmptyTitle>
              <EmptyDescription>
                Pages are created on your machine: tmt colab page create --title &quot;Notes&quot;
              </EmptyDescription>
            </EmptyHeader>
            <EmptyContent>
              <Button type="button">New page</Button>
            </EmptyContent>
          </Empty>
        ) : shown.length === 0 ? (
          <Empty>
            <EmptyHeader>
              <EmptyTitle>No pages match</EmptyTitle>
              <EmptyDescription>
                Nothing in this view matches the search and filters.
              </EmptyDescription>
            </EmptyHeader>
            <EmptyContent>
              <Button type="button" variant="outline" onClick={() => setQuery({ filter: null })}>
                Clear filters
              </Button>
            </EmptyContent>
          </Empty>
        ) : (
          <>
            <p className="list-quiet list-status">{shown.length} pages in this view</p>
            <Table shown={shown} />
          </>
        )}
        <section className="reader-notes">
          <h2>Missing parts</h2>
          <ul>
            {listMissing.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
        </section>
      </div>
      {query.get('clear') === '1' && (
        <Dialog open onOpenChange={(open) => !open && setQuery({ clear: null })}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Clear all Needs-you items?</DialogTitle>
              <DialogDescription>
                The 4 items leave the menu and the counts. Their pages and threads stay as they are.
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setQuery({ clear: null })}>
                Cancel
              </Button>
              <Button type="button" onClick={() => setQuery({ clear: null, needs: 'cleared' })}>
                Clear all
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
      <StatePanel controls={controls}>{globals}</StatePanel>
    </TooltipProvider>
  );
}
