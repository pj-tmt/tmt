import { ArrowDown, ArrowUp, Bookmark, Copy, GripVertical, Pencil, Trash2 } from 'lucide-react';
import type { ReactNode } from 'react';
import { BrowserTopBar, useTopBarReady } from '../src/react.js';
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  TooltipProvider,
} from '../src/ui/index.js';
import { StatePanel, type StateControl } from './mock-reader.js';
import { setQuery } from './router.js';

/*
 * The side menu's #5826 proposal C states, the shortcut Manage dialog and the Colab settings page
 * (colab-settings.tsx), in palette B and ui parts (#5821, GUI lead note 8).
 */

export const menuControls: StateControl[] = [
  {
    key: 'shortcuts',
    label: 'Shortcuts',
    options: [
      ['', 'Some'],
      ['none', 'None'],
    ],
  },
  {
    key: 'dots',
    label: 'Needs-you counts',
    options: [
      ['', 'On'],
      ['off', 'Off'],
    ],
  },
  {
    key: 'manage',
    label: 'Manage dialog',
    options: [
      ['', 'Closed'],
      ['1', 'Open'],
    ],
  },
];

const shortcutRows: [string, string][] = [
  ['spec', 'project:spec'],
  ['night', 'project:night-shift · needs me'],
  ['pm', 'project:pm'],
  ['design', 'project:design · stage:in-progress'],
  ['blog', 'project:blog'],
];

/** Manage shortcuts: order, edit and delete, in the order the menu shows them. */
export function ManageDialog() {
  return (
    <Dialog open onOpenChange={(open) => !open && setQuery({ manage: null })}>
      <DialogContent className="menu-manage">
        <DialogHeader>
          <DialogTitle>Manage shortcuts</DialogTitle>
          <DialogDescription>
            Order sets the menu. Save new ones from the list with Save as shortcut.
          </DialogDescription>
        </DialogHeader>
        <ul className="menu-shortcuts">
          {shortcutRows.map(([name, search], index) => (
            <li key={name}>
              <GripVertical className="menu-grip" aria-hidden="true" />
              <Bookmark className="menu-icon" aria-hidden="true" />
              <span className="menu-name">
                <b>{name}</b>
                <code>{search}</code>
              </span>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={`Move ${name} up`}
                disabled={index === 0}
              >
                <ArrowUp aria-hidden="true" />
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={`Move ${name} down`}
                disabled={index === shortcutRows.length - 1}
              >
                <ArrowDown aria-hidden="true" />
              </Button>
              <Button type="button" variant="ghost" size="icon" aria-label={`Edit ${name}`}>
                <Pencil aria-hidden="true" />
              </Button>
              <Button type="button" variant="ghost" size="icon" aria-label={`Delete ${name}`}>
                <Trash2 aria-hidden="true" />
              </Button>
            </li>
          ))}
        </ul>
        <DialogFooter>
          <Button type="button" onClick={() => setQuery({ manage: null })}>
            Done
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function SettingsBar() {
  return useTopBarReady() ? (
    <BrowserTopBar
      title={
        <h1>
          <span className="tmt-ui-topbar-title-prefix">Colab / </span>
          <span className="tmt-ui-topbar-title-current">Settings</span>
        </h1>
      }
    />
  ) : null;
}

const reference: [string, string, string][] = [
  ['Open in browser', 'Serve and page create open a browser.', 'On'],
  ['Work in progress', 'Assigned to-do and in-progress tasks across all pages.', '3 per agent'],
  ['GitHub repository', 'Where tmt colab task github pushes tasks.', 'Not set'],
  ['Page expiry', 'Days without an edit, comment or reply before archive.', '30 days'],
];

/** The Colab settings page: shortcuts, then the CLI-managed reference defaults. */
export function SettingsScreen({ globals }: { globals: ReactNode }) {
  return (
    <TooltipProvider>
      <SettingsBar />
      <div className="menu-settings">
        <header>
          <h2 className="menu-h2">Colab settings</h2>
          <p className="menu-quiet">Reference defaults and a clear path to your terminal.</p>
        </header>
        <Card>
          <CardHeader>
            <CardTitle>Shortcuts</CardTitle>
            <CardDescription>5 shortcuts in the side menu, in this order.</CardDescription>
          </CardHeader>
          <CardContent className="menu-row">
            <span className="menu-quiet">{shortcutRows.map(([name]) => name).join(' · ')}</span>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => setQuery({ manage: '1' })}
            >
              Manage
            </Button>
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle>
              Current settings <Badge variant="secondary">CLI managed</Badge>
            </CardTitle>
            <CardDescription>Read-only here. Change these with tmt colab settings.</CardDescription>
          </CardHeader>
          <CardContent>
            <dl className="menu-defs">
              {reference.map(([name, help, value]) => (
                <div key={name}>
                  <dt>
                    <b>{name}</b>
                    <span className="menu-quiet">{help}</span>
                  </dt>
                  <dd>{value}</dd>
                </div>
              ))}
            </dl>
            <div className="menu-command">
              <code>tmt colab settings</code>
              <Button type="button" variant="ghost" size="icon" aria-label="Copy the command">
                <Copy aria-hidden="true" />
              </Button>
            </div>
          </CardContent>
        </Card>
      </div>
      <StatePanel controls={menuControls}>{globals}</StatePanel>
    </TooltipProvider>
  );
}
