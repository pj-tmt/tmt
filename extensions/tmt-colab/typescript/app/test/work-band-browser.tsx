import '@tmt/browser-ui/static.css';
import '../src/style.css';
import { createRoot, type Root } from 'react-dom/client';
import { WorkBand, type WorkBandProps } from '../src/work-band.js';
import type { Work } from '../src/work.js';

const writerId = '10000000-0000-4000-8000-000000000001';
const attribution = { writerId, timeMs: 1_000, displayAgent: { name: 'Atlas' } };
const CHILD = '20000000-0000-4000-8000-000000000002';

const full: Work = {
  version: 1,
  next: 8,
  order: ['T-1', 'T-2', 'T-3', 'T-4', 'T-5', 'T-6', 'T-7'],
  summary: { line1: 'Building the export', line2: 'Two tasks left before review', attribution },
  tasks: {
    'T-1': { title: 'Write the contract', state: 'done' },
    'T-2': {
      title: 'Wire the export to the page menu',
      state: 'in-progress',
      assignee: { kind: 'agent', name: 'Atlas' },
      detail: 'Keep the menu entry\nbehind the existing More group.',
      acceptance: [
        { kind: 'test', name: 'export.spec' },
        { kind: 'check', command: 'pnpm check', result: 'passed' },
        { kind: 'person', who: 'Ben' },
      ],
    },
    'T-3': { title: 'Review the file format', state: 'in-review', needsSignoff: true },
    'T-4': { title: 'Choose the archive library', state: 'blocked', reason: 'Waiting on licence' },
    'T-5': {
      title: 'Add a progress line',
      state: 'todo',
      assignee: { kind: 'person', name: 'Ben' },
    },
    'T-6': { title: 'Follow-up page for sizes', state: 'todo', child: CHILD },
    'T-7': { title: 'Tracked elsewhere', state: 'todo', externalId: 'GH-123' },
  },
};
const summaryOnly: Work = {
  version: 1,
  next: 1,
  order: [],
  tasks: {},
  summary: { line1: 'Reading notes', line2: 'No tasks yet', attribution },
};
const empty: Work = { version: 1, next: 1, order: [], tasks: {} };
const cases: Record<string, WorkBandProps> = {
  full: {
    work: full,
    pageState: 'todo',
    progress: { done: 1, inProgress: 3, todo: 2, unknown: 1, total: 7 },
  },
  zero: {
    work: empty,
    pageState: 'todo',
    progress: { done: 0, inProgress: 0, todo: 0, unknown: 0, total: 0 },
  },
  complete: {
    work: empty,
    pageState: 'in-progress',
    progress: { done: 5, inProgress: 0, todo: 0, unknown: 0, total: 5 },
  },
  panel: {
    work: {
      version: 1,
      next: 6,
      order: ['T-1', 'T-2', 'T-3', 'T-4', 'T-5'],
      tasks: {
        'T-1': { title: 'Data and commands', state: 'todo', child: CHILD },
        'T-2': {
          title: 'Status panel',
          state: 'todo',
          child: '20000000-0000-4000-8000-000000000003',
        },
        'T-3': {
          title: 'Sidebar shortcuts',
          state: 'todo',
          child: '20000000-0000-4000-8000-000000000004',
        },
        'T-4': { title: 'Review the contract', state: 'done' },
        'T-5': { title: 'Check the captures', state: 'done' },
      },
    },
    pageState: 'in-progress',
    progress: { done: 3, inProgress: 1, todo: 1, unknown: 0, total: 5 },
    subpages: [
      { id: CHILD, title: 'Data and commands', state: 'done' },
      { id: '20000000-0000-4000-8000-000000000003', title: 'Status panel', state: 'in-progress' },
      { id: '20000000-0000-4000-8000-000000000004', title: 'Sidebar shortcuts', state: 'todo' },
    ],
  },
  parent: {
    work: full,
    parent: { pageId: CHILD, taskId: 'T-6', title: 'Follow-up page for sizes' },
  },
  'summary-only': { work: summaryOnly },
  empty: { work: empty },
};

let root: Root | undefined;
/** One band on the app's paper; `name` picks the work it shows. */
export function mount(name: keyof typeof cases = 'full', focusTask?: string) {
  document.getElementById('root')?.setAttribute('hidden', '');
  root?.unmount();
  document.getElementById('work-band-fixture')?.remove();
  const host = document.createElement('main');
  host.id = 'work-band-fixture';
  host.style.cssText = 'max-width:1200px;margin:0 auto';
  document.body.append(host);
  root = createRoot(host);
  const shown = cases[name]!;
  root.render(
    <>
      <WorkBand
        {...shown}
        focusTask={focusTask}
        pageHref={(pageId, hint) =>
          `#/pages/${pageId}${hint && 'task' in hint ? `?task=${hint.task}` : ''}`
        }
        actions={(_task, id) => <button type="button">Edit {id}</button>}
      />
      <p data-after>The page starts here.</p>
    </>,
  );
}
