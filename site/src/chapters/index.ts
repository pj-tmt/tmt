import type { MDXContent } from "mdx/types";
import type { Status } from "../components/marks";
import Colab from "./colab.mdx";
import Concepts from "./concepts.mdx";
import Design from "./design.mdx";
import DevDriver from "./dev-driver.mdx";
import DevExtension from "./dev-extension.mdx";
import Drivers from "./drivers.mdx";
import DrvClaude from "./drv-claude.mdx";
import DrvCodex from "./drv-codex.mdx";
import DrvTmux from "./drv-tmux.mdx";
import Extensions from "./extensions.mdx";
import Meet from "./meet.mdx";
import Remote from "./remote.mdx";
import Settings from "./settings.mdx";
import Squad from "./squad.mdx";
import Start from "./start.mdx";
import Working from "./working.mdx";

// A window in the status bar, like a tmux window: one per chapter group.
export type Window = { n: number; name: string; path: string };

export const windows: Window[] = [
  { n: 0, name: "home", path: "/" },
  { n: 1, name: "concepts", path: "/concepts" },
  { n: 2, name: "working", path: "/working" },
  { n: 3, name: "drivers", path: "/drivers" },
  { n: 4, name: "extensions", path: "/extensions" },
  { n: 5, name: "develop", path: "/develop/extensions" },
  { n: 6, name: "design", path: "/design" },
];

export type Page = {
  path: string;
  window: number;
  // The chapter number shown on the page's border rule, and its place in the tree.
  index?: string;
  crumb: string;
  title: string;
  status?: { kind: Status; label: string };
  // The English chapter file (src/chapters/<file>.mdx); a translation of the
  // page has the same name under src/i18n/<lang>/.
  file: string;
  Content: MDXContent;
};

export const pages: Page[] = [
  {
    path: "/",
    window: 0,
    crumb: "home",
    title: "One channel for all your agents",
    file: "start",
    Content: Start,
  },
  {
    path: "/concepts",
    window: 1,
    index: "1",
    crumb: "concepts",
    title: "Four words cover almost everything",
    file: "concepts",
    Content: Concepts,
  },
  {
    path: "/working",
    window: 2,
    index: "2",
    crumb: "working",
    title: "Launch, send, get the reply",
    file: "working",
    Content: Working,
  },
  {
    path: "/working/settings",
    window: 2,
    index: "2.1",
    crumb: "working / settings",
    title: "Settings and troubleshooting",
    file: "settings",
    Content: Settings,
  },
  {
    path: "/drivers",
    window: 3,
    index: "3",
    crumb: "drivers",
    title: "Drivers",
    file: "drivers",
    Content: Drivers,
  },
  {
    path: "/drivers/tmux",
    window: 3,
    index: "3.1",
    crumb: "drivers / tmux",
    title: "tmux: where your agents live",
    status: { kind: "built in", label: "built in" },
    file: "drv-tmux",
    Content: DrvTmux,
  },
  {
    path: "/drivers/claude-code",
    window: 3,
    index: "3.2",
    crumb: "drivers / claude code",
    title: "Claude Code",
    status: { kind: "built in", label: "built in" },
    file: "drv-claude",
    Content: DrvClaude,
  },
  {
    path: "/drivers/codex",
    window: 3,
    index: "3.3",
    crumb: "drivers / codex",
    title: "Codex",
    status: { kind: "built in", label: "built in" },
    file: "drv-codex",
    Content: DrvCodex,
  },
  {
    path: "/extensions",
    window: 4,
    index: "4",
    crumb: "extensions",
    title: "Extensions add commands, not special cases",
    file: "extensions",
    Content: Extensions,
  },
  {
    path: "/extensions/squad",
    window: 4,
    index: "4.1",
    crumb: "extensions / squad",
    title: "Ops: leads, members and one board",
    status: { kind: "alpha", label: "alpha" },
    file: "squad",
    Content: Squad,
  },
  {
    path: "/extensions/colab",
    window: 4,
    index: "4.2",
    crumb: "extensions / colab",
    title: "Colab: one page your whole team shares",
    status: { kind: "in progress", label: "in progress" },
    file: "colab",
    Content: Colab,
  },
  {
    path: "/extensions/meet",
    window: 4,
    index: "4.3",
    crumb: "extensions / meet",
    title: "Meet: a room for you and your agents",
    status: { kind: "planned", label: "planned" },
    file: "meet",
    Content: Meet,
  },
  {
    path: "/extensions/remote",
    window: 4,
    index: "4.4",
    crumb: "extensions / remote",
    title: "Remote: your agents, on every machine",
    status: { kind: "designing", label: "designing" },
    file: "remote",
    Content: Remote,
  },
  {
    path: "/develop/extensions",
    window: 5,
    index: "5.1",
    crumb: "develop / extensions",
    title: "Build an extension",
    file: "dev-extension",
    Content: DevExtension,
  },
  {
    path: "/develop/drivers",
    window: 5,
    index: "5.2",
    crumb: "develop / drivers",
    title: "Build a driver",
    status: { kind: "planned", label: "planned" },
    file: "dev-driver",
    Content: DevDriver,
  },
  {
    path: "/design",
    window: 6,
    index: "6",
    crumb: "design",
    title: "One look, everywhere",
    file: "design",
    Content: Design,
  },
];

// Links into the single-page handbook this site replaces (#squad, #drv-codex, …).
export const legacyAnchors: Record<string, string> = {
  start: "/",
  concepts: "/concepts",
  working: "/working",
  drivers: "/drivers",
  "drv-tmux": "/drivers/tmux",
  "drv-claude": "/drivers/claude-code",
  "drv-codex": "/drivers/codex",
  extensions: "/extensions",
  squad: "/extensions/squad",
  "squad-config": "/extensions/squad#make-it-yours",
  "squad-layout": "/extensions/squad#layout-rows-lines-and-panes",
  "squad-themes": "/extensions/squad#colors-and-themes",
  "squad-hosts": "/extensions/squad#jumping-between-members",
  threads: "/extensions",
  remote: "/extensions/remote",
  "dev-extension": "/develop/extensions",
  "dev-driver": "/develop/drivers",
};
