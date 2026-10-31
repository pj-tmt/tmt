import { useState, type CSSProperties } from 'react';
import { Info } from 'lucide-react';
import {
  Alert,
  AlertDescription,
  AlertTitle,
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
  Checkbox,
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Separator,
  Skeleton,
  Switch,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  Textarea,
  UiTheme,
} from '../src/ui/index.js';
import { BrowserAction, BrowserField, BrowserNotice, BrowserTabs } from '../src/react.js';
import { FramePair, Section } from './kit.js';
import './theme-b.css';

/** The radius switch (#5821): the one knob at 2px or 4px, or today's values for comparison. */
const radii = {
  '2': { '--tmt-ui-ref-radius': '2px' },
  '4': { '--tmt-ui-ref-radius': '4px' },
  current: {
    '--tmt-ui-radius-s': '6px',
    '--tmt-ui-radius': '8px',
    '--tmt-ui-radius-l': '12px',
    '--tmt-ui-radius-badge': '999px',
  },
} as const;
type Radius = keyof typeof radii;
const radiusLabels: Record<Radius, string> = { '2': '2 / 4', '4': '4 / 6', current: 'Current' };
const states = [
  ['working', '● Working'],
  ['waiting', '◆ Waiting'],
  ['review', '◐ Review'],
  ['blocked', '✕ Blocked'],
] as const;

/** Every part of the shadcn set, and the Browser* parts the Colab elements render, inside one
 * `[data-tmt-palette='b']` scope with a page-level theme and radius switch. */
export function ThemeBFrame({ params }: { params: URLSearchParams }) {
  const [theme, setTheme] = useState<'light' | 'dark'>(
    params.get('theme') === 'light' ? 'light' : 'dark',
  );
  const [radius, setRadius] = useState<Radius>(
    (['2', '4', 'current'] as const).find((key) => key === params.get('radius')) ?? '2',
  );
  return (
    <main
      className="theme-b"
      data-tmt-palette="b"
      data-theme={theme}
      data-radius={radius}
      style={radii[radius] as CSSProperties}
      aria-label="Theme B"
    >
      <div className="theme-b-switches" role="group" aria-label="Preview">
        <span>Theme</span>
        {(['dark', 'light'] as const).map((value) => (
          <button
            key={value}
            type="button"
            aria-pressed={theme === value}
            onClick={() => setTheme(value)}
          >
            {value === 'dark' ? 'Dark' : 'Light'}
          </button>
        ))}
        <span>Radius</span>
        {(Object.keys(radii) as Radius[]).map((value) => (
          <button
            key={value}
            type="button"
            aria-pressed={radius === value}
            onClick={() => setRadius(value)}
          >
            {radiusLabels[value]}
          </button>
        ))}
      </div>
      <UiTheme theme={theme}>
        <div className="theme-b-grid">
          <section aria-label="Buttons and badges">
            <div className="theme-b-row">
              <Button type="button">Send</Button>
              <Button type="button" variant="outline">
                New timer
              </Button>
              <Button type="button" variant="secondary">
                Settings
              </Button>
              <Button type="button" variant="ghost">
                Cancel
              </Button>
              <Button type="button" variant="destructive">
                Delete
              </Button>
              <Button type="button" disabled>
                Disabled
              </Button>
            </div>
            <div className="theme-b-row">
              <Badge>spec</Badge>
              <Badge variant="secondary">design</Badge>
              <Badge variant="outline">Private</Badge>
              <Badge variant="destructive">Blocked</Badge>
            </div>
            <div className="theme-b-row theme-b-states">
              {states.map(([tone, label]) => (
                <span key={tone} data-tone={tone}>
                  {label}
                </span>
              ))}
            </div>
          </section>
          <section aria-label="Fields">
            <Input aria-label="Search agents" placeholder="Search agents" />
            <Textarea aria-label="Message" defaultValue="Chip spacing: 8px or 12px?" />
            <div className="theme-b-row">
              <Select defaultValue="1h">
                <SelectTrigger aria-label="Range">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="1h">Range 1h</SelectItem>
                  <SelectItem value="1d">Range 1d</SelectItem>
                  <SelectItem value="7d">Range 7d</SelectItem>
                </SelectContent>
              </Select>
              <Switch defaultChecked aria-label="Notify me" />
              <Checkbox defaultChecked aria-label="Select" />
            </div>
          </section>
          <section aria-label="Tabs and alerts">
            <Tabs defaultValue="members">
              <TabsList variant="line">
                <TabsTrigger value="members">Members</TabsTrigger>
                <TabsTrigger value="timers">Timers</TabsTrigger>
                <TabsTrigger value="settings">Settings</TabsTrigger>
              </TabsList>
              <TabsContent value="members">3 members · 2 working</TabsContent>
            </Tabs>
            <Alert>
              <Info aria-hidden="true" />
              <AlertTitle>Method</AlertTitle>
              <AlertDescription>Figures are list prices for one day.</AlertDescription>
            </Alert>
            <Alert variant="destructive">
              <AlertTitle>✕ Blocked</AlertTitle>
              <AlertDescription>The build failed twice.</AlertDescription>
            </Alert>
          </section>
          <section aria-label="Card, skeleton and empty">
            <Card>
              <CardHeader>
                <CardTitle>#tmt-gui</CardTitle>
                <CardDescription>Lead tmt-gui-lead · 3 members</CardDescription>
              </CardHeader>
              <CardContent>
                <Separator />
              </CardContent>
              <CardFooter>
                <Button type="button" size="sm">
                  Open
                </Button>
              </CardFooter>
            </Card>
            <div className="theme-b-skeleton">
              <Skeleton className="theme-b-skeleton-line" />
              <Skeleton className="theme-b-skeleton-line theme-b-skeleton-short" />
            </div>
            <Empty>
              <EmptyHeader>
                <EmptyMedia>
                  <Info aria-hidden="true" />
                </EmptyMedia>
                <EmptyTitle>No timers yet</EmptyTitle>
                <EmptyDescription>A timer reminds a member on a schedule.</EmptyDescription>
              </EmptyHeader>
              <EmptyContent>
                <Button type="button" variant="outline" size="sm">
                  New timer
                </Button>
              </EmptyContent>
            </Empty>
          </section>
          <section aria-label="Browser parts the elements render">
            <BrowserTabs
              label="Channel"
              tabs={[
                {
                  id: 'members',
                  tabId: 'theme-b-tab-0',
                  panelId: 'theme-b-panel-0',
                  label: 'Members',
                },
                {
                  id: 'timers',
                  tabId: 'theme-b-tab-1',
                  panelId: 'theme-b-panel-1',
                  label: 'Timers',
                },
              ]}
              selected="members"
              onSelect={() => {}}
            >
              {(tab) => <p>{tab.label}</p>}
            </BrowserTabs>
            <BrowserField
              controlId="theme-b-name"
              label="Name"
              renderControl={(props) => <input {...props} defaultValue="sample-lead" />}
            />
            <div className="theme-b-row">
              <BrowserAction type="button" variant="primary" label="Send" onActivate={() => {}} />
              <BrowserAction
                type="button"
                variant="secondary"
                label="New timer"
                onActivate={() => {}}
              />
            </div>
            <BrowserNotice
              headingLevel="none"
              tone="blocked"
              announcement="none"
              mark="✕"
              stateLabel="Blocked"
              title="Blocked: sample-1"
              body="A required check failed twice."
            />
          </section>
        </div>
      </UiTheme>
    </main>
  );
}

export function ThemeBSection() {
  return (
    <Section id="theme-b">
      <FramePair
        title="Theme B · the shadcn set"
        note="Palette B and the radius knob inside [data-tmt-palette='b'] (#5821). The product outside the scope is unchanged."
        frame="theme-b"
        height={1500}
      />
    </Section>
  );
}
