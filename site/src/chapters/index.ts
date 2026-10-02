import type { MDXContent } from "mdx/types";
import type { Status } from "../components/marks";
import Concepts from "./concepts.mdx";
import Design from "./design.mdx";
import DevDriver from "./dev-driver.mdx";
import DevExtension from "./dev-extension.mdx";
import Drivers from "./drivers.mdx";
import DrvClaude from "./drv-claude.mdx";
import DrvCodex from "./drv-codex.mdx";
import DrvTmux from "./drv-tmux.mdx";
import Extensions from "./extensions.mdx";
import Remote from "./remote.mdx";
import Settings from "./settings.mdx";
import Squad from "./squad.mdx";
import Start from "./start.mdx";
import Threads from "./threads.mdx";
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
  Content: MDXContent;
};

export const pages: Page[] = [
  {
    path: "/",
    window: 0,
    crumb: "home",
    title: "One channel for all your agents",
    Content: Start,
  },
  {
    path: "/concepts",
    window: 1,
    index: "1",
    crumb: "concepts",
    title: "Four words cover almost everything",
    Content: Concepts,
  },
  {
    path: "/working",
    window: 2,
    index: "2",
    crumb: "working",
    title: "Launch, send, get the reply",
    Content: Working,
  },
  {
    path: "/working/settings",
    window: 2,
    index: "2.1",
    crumb: "working / settings",
    title: "Settings and troubleshooting",
    Content: Settings,
  },
  { path: "/drivers", window: 3, index: "3", crumb: "drivers", title: "Drivers", Content: Drivers },
  {
    path: "/drivers/tmux",
    window: 3,
    index: "3.1",
    crumb: "drivers / tmux",
    title: "tmux: where your agents live",
    status: { kind: "built in", label: "built in" },
    Content: DrvTmux,
  },
  {
    path: "/drivers/claude-code",
    window: 3,
    index: "3.2",
    crumb: "drivers / claude code",
    title: "Claude Code",
    status: { kind: "built in", label: "built in" },
    Content: DrvClaude,
  },
  {
    path: "/drivers/codex",
    window: 3,
    index: "3.3",
    crumb: "drivers / codex",
    title: "Codex",
    status: { kind: "built in", label: "built in" },
    Content: DrvCodex,
  },
  {
    path: "/extensions",
    window: 4,
    index: "4",
    crumb: "extensions",
    title: "Extensions add commands, not special cases",
    Content: Extensions,
  },
  {
    path: "/extensions/squad",
    window: 4,
    index: "4.1",
    crumb: "extensions / squad",
    title: "Squad: leads, members and one board",
    status: { kind: "alpha", label: "alpha" },
    Content: Squad,
  },
  {
    path: "/extensions/threads",
    window: 4,
    index: "4.2",
    crumb: "extensions / threads",
    title: "Threads: your team's conversations in one window",
    status: { kind: "planned", label: "coming later" },
    Content: Threads,
  },
  {
    path: "/extensions/remote",
    window: 4,
    index: "4.3",
    crumb: "extensions / remote",
    title: "Remote: your agents, on every machine",
    status: { kind: "designing", label: "designing" },
    Content: Remote,
  },
  {
    path: "/develop/extensions",
    window: 5,
    index: "5.1",
    crumb: "develop / extensions",
    title: "Build an extension",
    Content: DevExtension,
  },
  {
    path: "/develop/drivers",
    window: 5,
    index: "5.2",
    crumb: "develop / drivers",
    title: "Build a driver",
    status: { kind: "planned", label: "planned" },
    Content: DevDriver,
  },
  {
    path: "/design",
    window: 6,
    index: "6",
    crumb: "design",
    title: "One look, everywhere",
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
  threads: "/extensions/threads",
  remote: "/extensions/remote",
  "dev-extension": "/develop/extensions",
  "dev-driver": "/develop/drivers",
};
