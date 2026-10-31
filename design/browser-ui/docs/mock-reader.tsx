import { useState, type ReactNode } from 'react';
import {
  Check,
  ChevronDown,
  ChevronRight,
  FileText,
  GripVertical,
  ListTree,
  MessageCircle,
  MessageSquarePlus,
  MessagesSquare,
  MousePointer2,
  MoreHorizontal,
  Pin,
  Plus,
  X,
} from 'lucide-react';
import { BrowserTopBar, useTopBarReady } from '../src/react.js';
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  Command,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Input,
  Separator,
  Surface,
  Textarea,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '../src/ui/index.js';
import { setQuery, useQuery } from './router.js';

/*
 * The Colab reader, reconstructed (#5821, GUI lead note 9): today's reader on main (router.tsx's
 * floating actions, the floating discussion windows of panel-layout.css, annotation markers and the
 * selection composer, the question group card) plus Night's planned changes on Colab 36b98efb: the
 * #5506 sub nav, the #5820 page work band, the #5813 Chat dot and the #5817 window height. Every piece
 * is a design-system part; the ones that are not yet are listed under Missing parts.
 */

type Status = 'To do' | 'In progress' | 'Done';
interface SubPage {
  id: string;
  title: string;
  status: Status;
  progress?: string;
  children?: SubPage[];
}
const subPages: SubPage[] = [
  {
    id: 'remote-v2',
    title: 'Remote pages v2',
    status: 'In progress',
    progress: '2 of 4',
    children: [
      { id: 'invites', title: 'Invites tab', status: 'Done', progress: '1 of 1' },
      { id: 'machines', title: 'Machines list', status: 'In progress', progress: '1 of 3' },
    ],
  },
  { id: 'notes', title: 'Release notes draft', status: 'Done' },
  { id: 'host-log', title: 'Host upgrade log', status: 'To do' },
  { id: 'rollout', title: 'Rollout checklist', status: 'In progress', progress: '0 of 2' },
  { id: 'pairing', title: 'Pairing walkthrough', status: 'Done' },
];
const siblings: [string, Status][] = [
  ['Release plan', 'In progress'],
  ['Theme B review', 'In progress'],
  ['Release notes draft', 'Done'],
];

const tone: Record<Status, string> = { 'To do': 'todo', 'In progress': 'working', Done: 'done' };
function StatusChip({ status, onPick }: { status: Status; onPick?: () => void }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="reader-status"
          data-tone={tone[status]}
          onClick={(event) => {
            event.stopPropagation();
            onPick?.();
          }}
        >
          {status}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {(['To do', 'In progress', 'Done'] as Status[]).map((option) => (
          <DropdownMenuItem key={option}>
            {option === status && <Check aria-hidden="true" />}
            {option}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** The #5506 sub nav in the shared top bar: parent and its progress chip, › title, sub-pages. */
function SubNav({ parent, subs }: { parent: boolean; subs: boolean }) {
  return useTopBarReady() ? (
    <BrowserTopBar
      title={
        <div className="reader-subnav">
          {parent && (
            <>
              <ListTree className="reader-subnav-icon" aria-hidden="true" />
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button type="button" className="reader-parent">
                    <span className="reader-parent-name">Release plan</span>
                    <Badge variant="secondary">2/3</Badge>
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start" className="reader-menu">
                  <DropdownMenuItem>← Back to Release plan</DropdownMenuItem>
                  <DropdownMenuSeparator />
                  <DropdownMenuLabel>Release plan · 3 sub-pages</DropdownMenuLabel>
                  {siblings.map(([title, status]) => (
                    <DropdownMenuItem key={title}>
                      <span className="reader-menu-title" data-current={title === 'Theme B review'}>
                        {title}
                        {title === 'Theme B review' && ' (this page)'}
                      </span>
                      <span className="reader-status" data-tone={tone[status]}>
                        {status}
                      </span>
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
              <span className="reader-subnav-sep" aria-hidden="true">
                ›
              </span>
            </>
          )}
          <h1 className="reader-title">Theme B review</h1>
          {subs && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button type="button" variant="outline" size="xs" className="reader-subs">
                  <span className="reader-subs-long">5 sub-pages</span>
                  <span className="reader-subs-short">5</span>
                  <ChevronDown aria-hidden="true" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="reader-menu">
                {subPages.map((page) => (
                  <DropdownMenuItem key={page.id}>
                    <span className="reader-menu-title">{page.title}</span>
                    <span className="reader-status" data-tone={tone[page.status]}>
                      {page.status}
                    </span>
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
          <Badge variant="outline" className="reader-project">
            spec
          </Badge>
          <Button type="button" variant="ghost" size="icon" aria-label="Add project">
            <Plus aria-hidden="true" />
          </Button>
          <Button type="button" variant="ghost" size="icon" aria-label="Pin page">
            <Pin aria-hidden="true" />
          </Button>
        </div>
      }
      status={<span className="reader-quiet">Live</span>}
    />
  ) : null;
}

function TreeRows({ pages, depth }: { pages: SubPage[]; depth: number }) {
  const [open, setOpen] = useState<ReadonlySet<string>>(() => new Set(['remote-v2']));
  return (
    <>
      {pages.map((page) => (
        <div key={page.id}>
          <div
            className="reader-row"
            style={{
              paddingLeft: `calc(var(--tmt-ui-ref-space-2) + ${depth} * var(--tmt-ui-ref-space-5))`,
            }}
          >
            <GripVertical className="reader-grip" aria-hidden="true" />
            {page.children ? (
              <button
                type="button"
                className="reader-twisty"
                aria-expanded={open.has(page.id)}
                aria-label={`${open.has(page.id) ? 'Collapse' : 'Expand'} ${page.title}`}
                onClick={() =>
                  setOpen((now) => {
                    const next = new Set(now);
                    if (next.has(page.id)) next.delete(page.id);
                    else next.add(page.id);
                    return next;
                  })
                }
              >
                {open.has(page.id) ? <ChevronDown /> : <ChevronRight />}
              </button>
            ) : (
              <span className="reader-twisty" />
            )}
            <FileText className="reader-row-icon" aria-hidden="true" />
            <a className="reader-row-title" href="#page">
              {page.title}
            </a>
            <span className="reader-row-progress">{page.progress ?? '—'}</span>
            <StatusChip status={page.status} />
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="reader-row-more"
                  aria-label={`More for ${page.title}`}
                >
                  <MoreHorizontal aria-hidden="true" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem>Open</DropdownMenuItem>
                <DropdownMenuItem>Detach</DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
          {page.children && open.has(page.id) && (
            <TreeRows pages={page.children} depth={depth + 1} />
          )}
        </div>
      ))}
    </>
  );
}

/** The #5820 page work band, sticky under the nav: the whole bar toggles; its chip is its own menu. */
function WorkBand({ mode }: { mode: 'collapsed' | 'expanded' | 'empty' }) {
  const toggle = () => setQuery({ panel: mode === 'expanded' ? 'collapsed' : 'expanded' });
  if (mode === 'empty')
    return (
      <div className="reader-band" data-mode="empty">
        <div
          className="reader-band-bar"
          role="button"
          tabIndex={0}
          onClick={() => setQuery({ add: '1' })}
        >
          <StatusChip status="To do" />
          <span className="reader-quiet">No sub-pages yet</span>
          <Button
            type="button"
            variant="ghost"
            size="xs"
            onClick={(event) => {
              event.stopPropagation();
              setQuery({ add: '1' });
            }}
          >
            <Plus aria-hidden="true" /> Add
          </Button>
        </div>
      </div>
    );
  return (
    <div className="reader-band" data-mode={mode}>
      <div
        className="reader-band-bar"
        role="button"
        tabIndex={0}
        aria-expanded={mode === 'expanded'}
        onClick={toggle}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            toggle();
          }
        }}
      >
        <StatusChip status="In progress" />
        <span className="reader-progress" aria-hidden="true">
          <span style={{ width: '60%' }} />
        </span>
        <span className="reader-band-count">
          <span className="reader-subs-long">3 of 5 done</span>
          <span className="reader-subs-short">3/5</span>
        </span>
        <ChevronDown
          className="reader-band-chevron"
          data-open={mode === 'expanded'}
          aria-hidden="true"
        />
      </div>
      {mode === 'expanded' && (
        <div className="reader-tree">
          <TreeRows pages={subPages} depth={0} />
          <Separator />
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="reader-add"
            onClick={() => setQuery({ add: '1' })}
          >
            <Plus aria-hidden="true" /> Add
          </Button>
        </div>
      )}
    </div>
  );
}

/** The one Add dialog (#5820 Mock 5): search, a Create row, attachable pages, refused ones with why. */
function AddDialog() {
  const [query, setText] = useState('remote rollout');
  return (
    <Dialog open onOpenChange={(open) => !open && setQuery({ add: null })}>
      <DialogContent className="reader-add-dialog">
        <DialogHeader>
          <DialogTitle>Add a sub-page to “Theme B review”</DialogTitle>
          <DialogDescription>
            Search a page to attach, or create one with this name.
          </DialogDescription>
        </DialogHeader>
        <Command shouldFilter={false}>
          <CommandInput value={query} onValueChange={setText} placeholder="Search pages" />
          <CommandList>
            {query && (
              <CommandGroup>
                <CommandItem>
                  <Plus aria-hidden="true" /> Create “{query}” as a new sub-page
                </CommandItem>
              </CommandGroup>
            )}
            <CommandGroup heading="Pages">
              <CommandItem>
                Remote admin prototype <Badge variant="outline">spec</Badge>
              </CommandItem>
              <CommandItem>Remote data plan design</CommandItem>
              <CommandItem disabled>
                Remote pages v2 <span className="reader-why">Already under Release plan</span>
              </CommandItem>
              <CommandItem disabled>
                Release plan <span className="reader-why">Would loop: it is above this page</span>
              </CommandItem>
            </CommandGroup>
          </CommandList>
        </Command>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => setQuery({ add: null })}>
            Cancel
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** A numbered annotation marker after its range. Missing part: Marker. */
function Marker({
  n,
  selected,
  resolved,
}: {
  n: number | string;
  selected?: boolean;
  resolved?: boolean;
}) {
  return (
    <button
      type="button"
      className="reader-marker"
      data-selected={selected || undefined}
      data-resolved={resolved || undefined}
      aria-label={`Comment ${n}`}
      onClick={() => setQuery({ dock: 'comments', ann: 'selected' })}
    >
      {n}
    </button>
  );
}

function Annotated({
  children,
  n,
  on,
  selected,
  resolved,
}: {
  children: ReactNode;
  n: number;
  on: boolean;
  selected?: boolean;
  resolved?: boolean;
}) {
  if (!on) return <>{children}</>;
  return (
    <>
      <mark
        className="reader-range"
        data-selected={selected || undefined}
        data-resolved={resolved || undefined}
      >
        {children}
      </mark>
      <Marker n={n} selected={selected} resolved={resolved} />
    </>
  );
}

/** The selection and its round Annotate control (selection-control), inside the author's text. */
function Selection({ open }: { open: boolean }) {
  return (
    <span className="reader-selection-wrap">
      <span className="reader-selection">the selected tab and the primary action</span>
      {!open && (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              type="button"
              size="icon"
              className="reader-annotate"
              aria-label="Annotate"
              onClick={() => setQuery({ composer: '1' })}
            >
              <MessageSquarePlus aria-hidden="true" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Annotate</TooltipContent>
        </Tooltip>
      )}
    </span>
  );
}

/** The composer window under the selection (annotation-popover). */
function Composer() {
  return (
    <div className="reader-composer-anchor">
      <Surface level="overlay" className="reader-composer">
        <blockquote>the selected tab and the primary action</blockquote>
        <Textarea placeholder="Comment on this, or ask an agent…" aria-label="Comment" />
        <div className="reader-composer-foot">
          <span className="reader-quiet">Goes to</span>
          <Badge variant="outline">@sample-lead</Badge>
          <span className="reader-grow" />
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => setQuery({ composer: null })}
          >
            Cancel
          </Button>
          <Button type="button" size="sm" onClick={() => setQuery({ composer: null, ann: null })}>
            Comment
          </Button>
        </div>
      </Surface>
    </div>
  );
}

/** The question group card as the reader draws it (question-group-card.tsx). */
function Question({ answered }: { answered: boolean }) {
  const [choice, setChoice] = useState(0);
  if (answered)
    return (
      <Card className="reader-question" data-answered>
        <CardContent className="reader-qline">
          <MessagesSquare className="reader-qicon" aria-hidden="true" />
          <span className="reader-qhead">
            <span className="reader-eyebrow">Q&amp;A · sample-lead</span>
            <span className="reader-qtitle">Which radius ships with B?</span>
          </span>
          <span className="reader-qstate">
            <Check aria-hidden="true" /> Answered
          </span>
          <span className="reader-qanswer">2 / 4 (recommended)</span>
        </CardContent>
      </Card>
    );
  return (
    <Card className="reader-question">
      <CardHeader>
        <span className="reader-eyebrow">Q&amp;A · sample-lead · Needs your answer</span>
        <CardTitle>Which radius ships with B?</CardTitle>
      </CardHeader>
      <CardContent className="reader-qbody">
        <p className="reader-quiet">Single choice · Other available</p>
        <div className="reader-choices" role="radiogroup" aria-label="Which radius ships with B?">
          {['2 / 4 (recommended)', '4 / 6', 'Keep the current radius'].map((label, index) => (
            <button
              key={label}
              type="button"
              role="radio"
              aria-checked={choice === index}
              className="reader-choice"
              onClick={() => setChoice(index)}
            >
              <span className="reader-radio" aria-hidden="true" />
              {label}
              <kbd>{index + 1}</kbd>
            </button>
          ))}
        </div>
        <Textarea placeholder="Tell us what is missing…" aria-label="Your other answer" />
        <Separator />
        <div className="reader-qfoot">
          <span className="reader-quiet">Goes to</span>
          <Badge variant="outline">@sample-lead</Badge>
          <Button type="button" variant="ghost" size="icon" aria-label="Add recipient">
            <Plus aria-hidden="true" />
          </Button>
          <span className="reader-grow" />
          <Button type="button" onClick={() => setQuery({ q: 'answered' })}>
            Send answer
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

/** The answer batch card: answers kept on several questions, sent together. */
function AnswerBatch() {
  return (
    <Surface level="sheet" className="reader-batch">
      <div>
        <b>2 answers ready</b>
        <p className="reader-quiet">
          Radius · Default palette. They go to @sample-lead in one message.
        </p>
      </div>
      <Button type="button" variant="outline" size="sm">
        Review
      </Button>
      <Button type="button" size="sm">
        Send 2 answers
      </Button>
    </Surface>
  );
}

const threads = [
  {
    n: 1,
    quote: 'Corners are small: 2px on controls, 4px on surfaces.',
    resolved: false,
    turns: [
      ['sample-lead', '10:02', 'Agree on 2/4. Counts keep the same radius, no pills.'],
      ['sample-1', '10:05', 'Try 4/6 once on the gallery before we lock it.'],
    ],
  },
  {
    n: 2,
    quote: 'Status keeps muted hues with a mark and a word',
    resolved: false,
    turns: [['sample-2', '10:09', 'Is amber still the needs-you colour under B?']],
  },
  {
    n: 3,
    quote: 'Disabled is neutral grey on every variant',
    resolved: true,
    turns: [['sample-1', 'Yesterday', 'Done in the Button part.']],
  },
];

function Turn({ who, time, text }: { who: string; time: string; text: string }) {
  return (
    <div className="reader-turn">
      <span className="reader-avatar" aria-hidden="true">
        {who
          .split('-')
          .map((part) => part[0])
          .join('')
          .toUpperCase()}
      </span>
      <div>
        <p className="reader-who">
          {who} <span className="reader-quiet">{time}</span>
        </p>
        <p>{text}</p>
      </div>
    </div>
  );
}

/** The floating discussion window that opens out of the dock (#4919; height per #5817). */
function DockWindow({
  kind,
  selected,
  unread,
}: {
  kind: 'chat' | 'comments';
  selected: boolean;
  unread: boolean;
}) {
  return (
    <Surface
      level="overlay"
      className="reader-window"
      role="dialog"
      aria-label={kind === 'chat' ? 'Chat' : 'Comments'}
    >
      <header className="reader-window-bar">
        <b>{kind === 'chat' ? 'Chat' : 'Comments · 2 open'}</b>
        <span className="reader-grow" />
        <Button
          type="button"
          variant="ghost"
          size="icon"
          aria-label="Close"
          onClick={() => setQuery({ dock: null })}
        >
          <X aria-hidden="true" />
        </Button>
      </header>
      {kind === 'comments' ? (
        <div className="reader-window-body">
          {threads.map((thread) => (
            <section
              key={thread.n}
              className="reader-thread"
              data-selected={(selected && thread.n === 1) || undefined}
              data-resolved={thread.resolved || undefined}
            >
              <div className="reader-thread-head">
                <Marker
                  n={thread.n}
                  resolved={thread.resolved}
                  selected={selected && thread.n === 1}
                />
                <q className="reader-thread-quote">{thread.quote}</q>
                {thread.resolved && <Badge variant="secondary">Resolved</Badge>}
              </div>
              {(!thread.resolved || selected) &&
                thread.turns.map(([who, time, text]) => (
                  <Turn key={text} who={who!} time={time!} text={text!} />
                ))}
              {!thread.resolved && (
                <Input placeholder="Reply" aria-label={`Reply to comment ${thread.n}`} />
              )}
            </section>
          ))}
        </div>
      ) : (
        <div className="reader-window-body">
          <Turn
            who="sample-1"
            time="10:12"
            text="Looks much calmer. What happens to the amber counts?"
          />
          <Turn who="sample-2" time="10:14" text="They stay amber: status is never the accent." />
          {unread && <p className="reader-new">New</p>}
          <Turn
            who="sample-lead"
            time="10:15"
            text="Ship B as the default once the Colab pages are swept."
          />
        </div>
      )}
      {kind === 'chat' && (
        <footer className="reader-window-foot">
          <Textarea placeholder="Message @sample-lead…" aria-label="Message" />
          <Button type="button" size="sm">
            Send
          </Button>
        </footer>
      )}
    </Surface>
  );
}

/** The floating actions capsule (router.tsx): select mode, Chat, Comments with the open count. */
function Fab({ dock, unread }: { dock: string | null; unread: boolean }) {
  const action = (
    label: string,
    icon: ReactNode,
    expanded: boolean,
    onClick: () => void,
    extra?: ReactNode,
  ) => (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          className="reader-fab-button"
          aria-label={label}
          aria-expanded={expanded}
          onClick={onClick}
        >
          {icon}
          {extra}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="top">{label}</TooltipContent>
    </Tooltip>
  );
  return (
    <Surface level="overlay" className="reader-fab">
      {action('Select to comment', <MousePointer2 aria-hidden="true" />, false, () =>
        setQuery({ composer: '1' }),
      )}
      {action(
        unread ? 'Chat · Unread' : 'Chat',
        <MessageCircle aria-hidden="true" />,
        dock === 'chat',
        () => setQuery({ dock: dock === 'chat' ? null : 'chat' }),
        unread ? <span className="reader-dot" aria-hidden="true" /> : undefined,
      )}
      {action(
        'Comments (2)',
        <MessagesSquare aria-hidden="true" />,
        dock === 'comments',
        () => setQuery({ dock: dock === 'comments' ? null : 'comments' }),
        <span className="reader-fab-count">2</span>,
      )}
    </Surface>
  );
}

const controls: { key: string; label: string; options: [string, string][] }[] = [
  {
    key: 'dock',
    label: 'Dock',
    options: [
      ['', 'Closed'],
      ['chat', 'Chat'],
      ['comments', 'Comments'],
    ],
  },
  {
    key: 'ann',
    label: 'Annotations',
    options: [
      ['none', 'None'],
      ['', 'Some'],
      ['selected', 'One selected'],
    ],
  },
  {
    key: 'composer',
    label: 'Composer',
    options: [
      ['', 'Closed'],
      ['1', 'Open'],
    ],
  },
  {
    key: 'unread',
    label: 'Chat unread',
    options: [
      ['', 'Off'],
      ['1', 'On'],
    ],
  },
  {
    key: 'parent',
    label: 'Parent',
    options: [
      ['', 'Yes'],
      ['0', 'No'],
    ],
  },
  {
    key: 'subs',
    label: 'Sub-pages',
    options: [
      ['', 'Some'],
      ['0', 'None'],
    ],
  },
  {
    key: 'panel',
    label: 'Work band',
    options: [
      ['', 'Collapsed'],
      ['expanded', 'Expanded'],
      ['empty', 'No sub-pages'],
    ],
  },
  {
    key: 'add',
    label: 'Add dialog',
    options: [
      ['', 'Closed'],
      ['1', 'Open'],
    ],
  },
  {
    key: 'q',
    label: 'Question',
    options: [
      ['', 'Unanswered'],
      ['answered', 'Answered'],
    ],
  },
];

/** The mock's state drawer (lead note 8): each state is a query value, so links reproduce it. */
export function StatePanel({ children }: { children?: ReactNode }) {
  const query = useQuery();
  const [open, setOpen] = useState(false);
  return (
    <Surface level="overlay" className="reader-states" data-open={open || undefined}>
      <button
        type="button"
        className="reader-states-head"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        <b>States</b>
        <ChevronDown data-open={open} aria-hidden="true" />
      </button>
      {open && (
        <div className="reader-states-body">
          {controls.map((control) => (
            <div key={control.key} className="reader-states-row">
              <span className="reader-quiet">{control.label}</span>
              <div className="reader-states-options">
                {control.options.map(([value, label]) => {
                  const pressed = (query.get(control.key) ?? '') === value;
                  return (
                    <Button
                      key={value}
                      type="button"
                      size="xs"
                      variant={pressed ? 'secondary' : 'ghost'}
                      aria-pressed={pressed}
                      onClick={() => setQuery({ [control.key]: value || null })}
                    >
                      {label}
                    </Button>
                  );
                })}
              </div>
            </div>
          ))}
          {children}
        </div>
      )}
    </Surface>
  );
}

const missing = [
  'Annotation marker (numbered circle and +n pill after a range): a styled button here.',
  'Highlighted range: a mark with the annotation highlight tokens.',
  'Floating actions capsule (FAB dock): ghost Buttons on an overlay Surface with the pill radius.',
  'Discussion window (dock window): an overlay Surface placed from the nav to the dock.',
  'Status chip with its own menu: a DropdownMenu trigger styled as a chip.',
  'Progress bar: a token-filled span.',
  'RadioGroup for single-choice questions: buttons with role=radio.',
  'Avatar: initials in a span.',
  'Tree row (twisty, indent, drag handle, columns): plain rows here.',
];

export function ReaderScreen({ globals }: { globals: ReactNode }) {
  const query = useQuery();
  const dock = query.get('dock');
  const ann = query.get('ann') ?? 'some';
  const composer = query.get('composer') === '1';
  const unread = query.get('unread') === '1';
  const panel = (query.get('panel') ?? 'collapsed') as 'collapsed' | 'expanded' | 'empty';
  const annotations = ann !== 'none';
  const selected = ann === 'selected';
  return (
    <TooltipProvider>
      <SubNav
        parent={query.get('parent') !== '0'}
        subs={query.get('subs') !== '0' && panel !== 'empty'}
      />
      <WorkBand mode={query.get('subs') === '0' ? 'empty' : panel} />
      <div className="reader-page">
        <article className="tmt-ui-prose reader-body">
          <h1>Theme B review</h1>
          <p>
            Palette B moves every surface to one warm neutral family and keeps red for four jobs:
            the brand, focus, <Selection open={composer} />. This page asks for one decision before
            B becomes the default.
          </p>
          {composer && <Composer />}
          <h2>What changes</h2>
          <ul>
            <li>Surfaces read the ref tier; a theme switch only remaps roles.</li>
            <li>
              <Annotated n={2} on={annotations}>
                Status keeps muted hues with a mark and a word
              </Annotated>
              : <code>◆ Waiting</code>, <code>✕ Blocked</code>.
            </li>
            <li>
              <Annotated n={3} on={annotations} resolved>
                Disabled is neutral grey on every variant
              </Annotated>
              , never a faded red.
            </li>
          </ul>
          <blockquote>
            <Annotated n={1} on={annotations} selected={selected}>
              Corners are small: 2px on controls, 4px on surfaces.
            </Annotated>
          </blockquote>
          <h2>Decision</h2>
        </article>
        <div className="reader-blocks">
          <Question answered={query.get('q') === 'answered'} />
          {query.get('q') !== 'answered' && <AnswerBatch />}
        </div>
        <section className="reader-notes">
          <h2>Missing parts</h2>
          <ul>
            {missing.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
          <h2>Today</h2>
          <p className="reader-quiet">
            Reference captures of main&apos;s reader (chrome, chat-dock, comments-dock, annotation
            look and badge, answer batch card, column bands) follow from the Colab e2e specs at
            10:00 JST under the night rule.
          </p>
        </section>
      </div>
      {dock === 'chat' || dock === 'comments' ? (
        <DockWindow kind={dock} selected={selected} unread={unread} />
      ) : null}
      <Fab dock={dock} unread={unread && dock !== 'chat'} />
      {query.get('add') === '1' && <AddDialog />}
      <StatePanel>{globals}</StatePanel>
    </TooltipProvider>
  );
}
