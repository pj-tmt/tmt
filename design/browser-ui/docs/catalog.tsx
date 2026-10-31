import { useState, type ReactNode } from 'react';
import { Info, Search } from 'lucide-react';
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
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
  Input,
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Separator,
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
  Skeleton,
  Switch,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  Textarea,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '../src/ui/index.js';

export interface ComponentDoc {
  slug: string;
  name: string;
  description: string;
  preview(): ReactNode;
  usage: string;
  variants: [string, () => ReactNode][];
  dos?: ['do' | 'dont', string][];
}

const row = (...children: ReactNode[]) => <div className="docs-row">{children}</div>;
const importLine = (...names: string[]) =>
  `import { ${names.join(', ')} } from '@tmt/browser-ui/ui';\n\n`;

function SwitchDemo() {
  const [on, setOn] = useState(true);
  return (
    <label className="docs-inline">
      <Switch checked={on} onCheckedChange={setOn} aria-label="Notify me" /> Notify me when a member
      is blocked
    </label>
  );
}

export const components: ComponentDoc[] = [
  {
    slug: 'button',
    name: 'Button',
    description:
      'Starts an action. One primary button per surface takes the accent; everything else is outline, secondary or ghost.',
    preview: () =>
      row(
        <Button type="button" key="p">
          Send
        </Button>,
        <Button type="button" variant="outline" key="o">
          New timer
        </Button>,
        <Button type="button" variant="ghost" key="g">
          Cancel
        </Button>,
      ),
    usage: `${importLine('Button')}<Button type="button">Send</Button>\n<Button type="button" variant="outline">New timer</Button>`,
    variants: [
      [
        'Variants',
        () =>
          row(
            ...(['default', 'outline', 'secondary', 'ghost', 'destructive', 'link'] as const).map(
              (variant) => (
                <Button type="button" variant={variant} key={variant}>
                  {variant === 'default' ? 'Primary' : variant[0]!.toUpperCase() + variant.slice(1)}
                </Button>
              ),
            ),
          ),
      ],
      [
        'Sizes',
        () =>
          row(
            ...(['xs', 'sm', 'default', 'lg'] as const).map((size) => (
              <Button type="button" variant="outline" size={size} key={size}>
                {size}
              </Button>
            )),
          ),
      ],
      [
        'Disabled and busy',
        () =>
          row(
            <Button type="button" disabled key="d">
              Disabled
            </Button>,
            <Button type="button" variant="outline" disabled key="od">
              Disabled
            </Button>,
            <Button type="button" busy busyLabel="Sending" key="b" />,
          ),
      ],
    ],
    dos: [
      ['do', 'Use one primary button for the action the surface exists for.'],
      [
        'dont',
        'Fill a destructive action with red. Delete is outlined text in the blocked colour.',
      ],
    ],
  },
  {
    slug: 'badge',
    name: 'Badge',
    description:
      'A short label for a kind, audience or state. Neutral by default; a state badge carries its mark and word.',
    preview: () =>
      row(
        <Badge key="a">spec</Badge>,
        <Badge variant="outline" key="b">
          Private
        </Badge>,
        <Badge variant="destructive" key="c">
          ✕ Blocked
        </Badge>,
      ),
    usage: `${importLine('Badge')}<Badge>spec</Badge>\n<Badge variant="destructive">✕ Blocked</Badge>`,
    variants: [
      [
        'Variants',
        () =>
          row(
            ...(['default', 'secondary', 'outline', 'ghost', 'destructive'] as const).map(
              (variant) => (
                <Badge variant={variant} key={variant}>
                  {variant === 'destructive' ? '✕ Blocked' : variant}
                </Badge>
              ),
            ),
          ),
      ],
    ],
    dos: [
      ['do', 'Pair a state with its mark: ✕ Blocked, ◆ Waiting.'],
      ['dont', 'Fill a badge with the accent red; red fills belong to the primary action.'],
    ],
  },
  {
    slug: 'tabs',
    name: 'Tabs',
    description:
      'Switches between views of the same object. The selected tab carries the accent underline; its text stays the foreground.',
    preview: () => (
      <Tabs defaultValue="members">
        <TabsList variant="line">
          <TabsTrigger value="members">Members</TabsTrigger>
          <TabsTrigger value="timers">Timers</TabsTrigger>
          <TabsTrigger value="settings">Settings</TabsTrigger>
        </TabsList>
        <TabsContent value="members">3 members · 2 working</TabsContent>
        <TabsContent value="timers">2 timers</TabsContent>
        <TabsContent value="settings">Channel settings</TabsContent>
      </Tabs>
    ),
    usage: `${importLine('Tabs', 'TabsList', 'TabsTrigger', 'TabsContent')}<Tabs defaultValue="members">\n  <TabsList variant="line">\n    <TabsTrigger value="members">Members</TabsTrigger>\n    <TabsTrigger value="timers">Timers</TabsTrigger>\n  </TabsList>\n  <TabsContent value="members">…</TabsContent>\n</Tabs>`,
    variants: [
      [
        'Boxed',
        () => (
          <Tabs defaultValue="all">
            <TabsList>
              <TabsTrigger value="all">All</TabsTrigger>
              <TabsTrigger value="needs">Needs you</TabsTrigger>
              <TabsTrigger value="done">Done</TabsTrigger>
            </TabsList>
          </Tabs>
        ),
      ],
    ],
  },
  {
    slug: 'input',
    name: 'Input',
    description:
      'A single-line field. Its frame is the soft rule; focus recolours the frame in the accent.',
    preview: () => (
      <div className="docs-stack">
        <Input aria-label="Search agents" placeholder="Search agents" />
      </div>
    ),
    usage: `${importLine('Input')}<Input aria-label="Search agents" placeholder="Search agents" />`,
    variants: [
      [
        'States',
        () => (
          <div className="docs-stack">
            <Input aria-label="Name" defaultValue="sample-lead" />
            <Input aria-label="Invalid name" aria-invalid defaultValue="sample lead" />
            <Input aria-label="Disabled" disabled defaultValue="Locked" />
          </div>
        ),
      ],
    ],
  },
  {
    slug: 'textarea',
    name: 'Textarea',
    description: 'A multi-line field for messages and notes.',
    preview: () => (
      <div className="docs-stack">
        <Textarea aria-label="Message" placeholder="Message #sample-squad" />
      </div>
    ),
    usage: `${importLine('Textarea')}<Textarea aria-label="Message" placeholder="Message #sample-squad" />`,
    variants: [],
  },
  {
    slug: 'select',
    name: 'Select',
    description:
      'Picks one value from a short list. The trigger is a field; the list opens as a popper under it.',
    preview: () => (
      <Select defaultValue="1d">
        <SelectTrigger aria-label="Range">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="1h">Range 1h</SelectItem>
          <SelectItem value="1d">Range 1d</SelectItem>
          <SelectItem value="7d">Range 7d</SelectItem>
        </SelectContent>
      </Select>
    ),
    usage: `${importLine('Select', 'SelectTrigger', 'SelectValue', 'SelectContent', 'SelectItem')}<Select defaultValue="1d">\n  <SelectTrigger aria-label="Range"><SelectValue /></SelectTrigger>\n  <SelectContent>\n    <SelectItem value="1h">Range 1h</SelectItem>\n    <SelectItem value="1d">Range 1d</SelectItem>\n  </SelectContent>\n</Select>`,
    variants: [],
    dos: [['dont', 'Use a native select; the list must take the tokens and the theme.']],
  },
  {
    slug: 'checkbox',
    name: 'Checkbox',
    description: 'Turns one option on or off inside a form; checked takes the accent.',
    preview: () =>
      row(
        <label className="docs-inline" key="a">
          <Checkbox defaultChecked aria-label="Show comments" /> Show comments
        </label>,
        <label className="docs-inline" key="b">
          <Checkbox aria-label="Archived" /> Archived
        </label>,
        <label className="docs-inline" key="c">
          <Checkbox disabled aria-label="Locked" /> Locked
        </label>,
      ),
    usage: `${importLine('Checkbox')}<Checkbox defaultChecked aria-label="Show comments" />`,
    variants: [],
  },
  {
    slug: 'switch',
    name: 'Switch',
    description: 'Turns a setting on or off at once, without a save step.',
    preview: () => <SwitchDemo />,
    usage: `${importLine('Switch')}<Switch checked={on} onCheckedChange={setOn} aria-label="Notify me" />`,
    variants: [],
  },
  {
    slug: 'dialog',
    name: 'Dialog',
    description:
      'A modal question that blocks the page until it is answered. Keep it to one decision.',
    preview: () => (
      <Dialog>
        <DialogTrigger asChild>
          <Button type="button" variant="outline">
            Move to project
          </Button>
        </DialogTrigger>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Move to project</DialogTitle>
            <DialogDescription>Pick where this page lives.</DialogDescription>
          </DialogHeader>
          <Input aria-label="Project" defaultValue="Release planning" />
          <DialogFooter>
            <DialogClose asChild>
              <Button type="button" variant="outline">
                Cancel
              </Button>
            </DialogClose>
            <DialogClose asChild>
              <Button type="button">Move</Button>
            </DialogClose>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    ),
    usage: `${importLine('Dialog', 'DialogTrigger', 'DialogContent', 'DialogHeader', 'DialogTitle')}<Dialog>\n  <DialogTrigger asChild><Button type="button">Move</Button></DialogTrigger>\n  <DialogContent>\n    <DialogHeader><DialogTitle>Move to project</DialogTitle></DialogHeader>\n  </DialogContent>\n</Dialog>`,
    variants: [],
    dos: [['do', 'Name the action on the confirm button: “Move”, not “OK”.']],
  },
  {
    slug: 'sheet',
    name: 'Sheet',
    description: 'A drawer from the edge for settings and detail that keep the page in view.',
    preview: () => (
      <Sheet>
        <SheetTrigger asChild>
          <Button type="button" variant="outline">
            Page settings
          </Button>
        </SheetTrigger>
        <SheetContent side="right">
          <SheetHeader>
            <SheetTitle>Page settings</SheetTitle>
            <SheetDescription>Drafts stay here.</SheetDescription>
          </SheetHeader>
          <div className="docs-sheet-body">
            <Input aria-label="Title" defaultValue="Release plan" />
          </div>
          <SheetFooter>
            <SheetClose asChild>
              <Button type="button">Save</Button>
            </SheetClose>
          </SheetFooter>
        </SheetContent>
      </Sheet>
    ),
    usage: `${importLine('Sheet', 'SheetTrigger', 'SheetContent')}<Sheet>\n  <SheetTrigger asChild><Button type="button">Open</Button></SheetTrigger>\n  <SheetContent side="right">…</SheetContent>\n</Sheet>`,
    variants: [],
  },
  {
    slug: 'dropdown-menu',
    name: 'Dropdown menu',
    description: 'A short list of commands on one object, opened from a button.',
    preview: () => (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button type="button" variant="outline">
            Page actions
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuLabel>This page</DropdownMenuLabel>
          <DropdownMenuItem>Rename</DropdownMenuItem>
          <DropdownMenuItem>Move to project</DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive">Delete</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    ),
    usage: `${importLine('DropdownMenu', 'DropdownMenuTrigger', 'DropdownMenuContent', 'DropdownMenuItem')}<DropdownMenu>\n  <DropdownMenuTrigger asChild><Button type="button">Page actions</Button></DropdownMenuTrigger>\n  <DropdownMenuContent><DropdownMenuItem>Rename</DropdownMenuItem></DropdownMenuContent>\n</DropdownMenu>`,
    variants: [],
  },
  {
    slug: 'popover',
    name: 'Popover',
    description: 'Detail or a small form anchored to its trigger; it does not block the page.',
    preview: () => (
      <Popover>
        <PopoverTrigger asChild>
          <Button type="button" variant="outline">
            Details
          </Button>
        </PopoverTrigger>
        <PopoverContent align="start">
          <PopoverHeader>
            <PopoverTitle>Release plan</PopoverTitle>
            <PopoverDescription>Updated 2 minutes ago by core-ops.</PopoverDescription>
          </PopoverHeader>
        </PopoverContent>
      </Popover>
    ),
    usage: `${importLine('Popover', 'PopoverTrigger', 'PopoverContent')}<Popover>\n  <PopoverTrigger asChild><Button type="button">Details</Button></PopoverTrigger>\n  <PopoverContent>…</PopoverContent>\n</Popover>`,
    variants: [],
  },
  {
    slug: 'tooltip',
    name: 'Tooltip',
    description: 'Names an icon-only control on hover and focus. Never the only place for content.',
    preview: () => (
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button type="button" variant="outline" size="icon" aria-label="Search">
              <Search aria-hidden="true" />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="bottom">Search pages</TooltipContent>
        </Tooltip>
      </TooltipProvider>
    ),
    usage: `${importLine('Tooltip', 'TooltipProvider', 'TooltipTrigger', 'TooltipContent')}<Tooltip>\n  <TooltipTrigger asChild>…</TooltipTrigger>\n  <TooltipContent>Search pages</TooltipContent>\n</Tooltip>`,
    variants: [],
  },
  {
    slug: 'alert',
    name: 'Alert',
    description:
      'A callout in the flow of the page: a method note, a caveat or a blocking problem.',
    preview: () => (
      <div className="docs-stack">
        <Alert>
          <Info aria-hidden="true" />
          <AlertTitle>Method</AlertTitle>
          <AlertDescription>Figures are list prices for one day of use.</AlertDescription>
        </Alert>
      </div>
    ),
    usage: `${importLine('Alert', 'AlertTitle', 'AlertDescription')}<Alert>\n  <AlertTitle>Method</AlertTitle>\n  <AlertDescription>…</AlertDescription>\n</Alert>`,
    variants: [
      [
        'Destructive',
        () => (
          <div className="docs-stack">
            <Alert variant="destructive">
              <AlertTitle>✕ Blocked</AlertTitle>
              <AlertDescription>The required check failed twice.</AlertDescription>
            </Alert>
          </div>
        ),
      ],
    ],
  },
  {
    slug: 'command',
    name: 'Command',
    description: 'A searchable list of commands or destinations, the base of quick switchers.',
    preview: () => (
      <div className="docs-command">
        <Command>
          <CommandInput placeholder="Search pages and members" />
          <CommandList>
            <CommandEmpty>No results.</CommandEmpty>
            <CommandGroup heading="Pages">
              <CommandItem>Release plan</CommandItem>
              <CommandItem>Design system docs</CommandItem>
            </CommandGroup>
            <CommandGroup heading="Members">
              <CommandItem>sample-1</CommandItem>
              <CommandItem>sample-lead</CommandItem>
            </CommandGroup>
          </CommandList>
        </Command>
      </div>
    ),
    usage: `${importLine('Command', 'CommandInput', 'CommandList', 'CommandItem')}<Command>\n  <CommandInput placeholder="Search" />\n  <CommandList><CommandItem>Release plan</CommandItem></CommandList>\n</Command>`,
    variants: [],
  },
  {
    slug: 'separator',
    name: 'Separator',
    description: 'A hairline between groups of content.',
    preview: () => (
      <div className="docs-stack">
        <span>Members</span>
        <Separator />
        <span>Timers</span>
      </div>
    ),
    usage: `${importLine('Separator')}<Separator />`,
    variants: [],
  },
  {
    slug: 'card',
    name: 'Card',
    description:
      'A resting surface for one object: the sheet fill, the rule and the surface radius, with no shadow.',
    preview: () => (
      <Card className="docs-card-demo">
        <CardHeader>
          <CardTitle>#sample-squad</CardTitle>
          <CardDescription>Lead sample-lead · 3 members · 2 working</CardDescription>
        </CardHeader>
        <CardContent>
          <p className="docs-muted">Spec revision is waiting on you.</p>
        </CardContent>
        <CardFooter>
          <Button type="button" size="sm">
            Open
          </Button>
          <Button type="button" size="sm" variant="ghost">
            Mute
          </Button>
        </CardFooter>
      </Card>
    ),
    usage: `${importLine('Card', 'CardHeader', 'CardTitle', 'CardContent')}<Card>\n  <CardHeader><CardTitle>#sample-squad</CardTitle></CardHeader>\n  <CardContent>…</CardContent>\n</Card>`,
    variants: [],
    dos: [['dont', 'Add a shadow to a resting card; only floating layers cast one.']],
  },
  {
    slug: 'skeleton',
    name: 'Skeleton',
    description:
      'A quiet placeholder while content loads. It does not pulse; a busy action uses the busy mark.',
    preview: () => (
      <div className="docs-stack">
        <Skeleton className="docs-skeleton" />
        <Skeleton className="docs-skeleton docs-skeleton-short" />
      </div>
    ),
    usage: `${importLine('Skeleton')}<Skeleton className="h-3 w-40" />`,
    variants: [],
  },
  {
    slug: 'empty',
    name: 'Empty',
    description: 'What a list shows when it has nothing yet, with one way forward.',
    preview: () => (
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
    ),
    usage: `${importLine('Empty', 'EmptyHeader', 'EmptyTitle', 'EmptyDescription')}<Empty>\n  <EmptyHeader><EmptyTitle>No timers yet</EmptyTitle></EmptyHeader>\n</Empty>`,
    variants: [],
  },
];
