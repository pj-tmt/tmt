// The words of the site's own components (the home page, the status bar and
// the notes around a page), as typed data. English is the source. A language
// overrides any of it, key by key, in src/i18n/<lang>/strings.json; whatever it
// leaves out stays English. A string may contain `code` and *emphasis*, which
// <Inline> renders. Commands and sample terminal output stay in the scenes.
export const english = {
  ui: {
    notTranslated: "Not yet translated. This page is shown in English.",
    language: "Language",
    chapterWindows: "Chapter windows",
    allChapters: "All chapters",
    closeMenu: "Close menu",
    onThisPage: "On this page",
    pageNavigation: "Page navigation",
    previous: "← previous",
    next: "next →",
  },
  chrome: {
    crumbs: {
      start: "home",
      concepts: "concepts",
      working: "working",
      settings: "working / settings",
      drivers: "drivers",
      "drv-tmux": "drivers / tmux",
      "drv-claude": "drivers / claude code",
      "drv-codex": "drivers / codex",
      extensions: "extensions",
      squad: "extensions / squad",
      colab: "extensions / colab",
      meet: "extensions / meet",
      remote: "extensions / remote",
      "dev-extension": "develop / extensions",
      "dev-driver": "develop / drivers",
      design: "design",
    },
    status: {
      shipped: "shipped",
      alpha: "alpha",
      planned: "planned",
      designing: "designing",
      "in progress": "in progress",
      "built in": "built in",
    },
  },
  home: {
    eyebrow: "home",
    title: "One simple core. *Endless* ways for AI to work together.",
    tagline: "Dead simple agent-to-agent communication. Built to compose.",
    lede: "Mix agents from different tools. Claude Code hands a task to Codex, or to any agent that can run a command, and gets the answer back. tmt keeps a receipt that ties each reply to its request. It adds a layer and replaces nothing: keep your terminal, your agents and your accounts. The board, colab and meet are built on that one core.",
    worksWith: "Works with",
    agents: "agents",
    anyAgent: "any CLI agent",
    hosts: "runs in",
    next: "next",
    yourHarness: "your harness",
    yourServer: "your server",
    planned: "planned",
    designing: "designing",
    install: "Install",
    start: "Start with one message ↓",
    // The hero scene: a request from Claude Code to Codex and the reply back.
    handoff: {
      label:
        "A request between two providers: the reviewer, running Claude Code, asks the builder, running Codex, to review a diff with tmt talk. The request reaches Codex, which answers with tmt reply and the receipt it was given, and the reply lands back in Claude Code. The request ID and the receipt tie the two.",
      title: "reviewer (Claude Code) ⇄ builder (Codex)",
      reviewer: "reviewer",
      builder: "builder",
      request: "request",
      receipt: "receipt",
      reply: "reply",
      tied: "the receipt ties the reply to its request",
      result: "result",
      spatial: "3D exchange",
      flat: "static exchange",
      steps: "Exchange steps",
      play: "Play",
      pause: "Pause",
      local: "TMT / LOCAL EXCHANGE",
      note: "Illustrative output · Claude Code + Codex · tmux or Herdr",
    },
    // The start chapter's three concepts, mapped to the commands that make them.
    concepts: {
      items: [
        {
          name: "agent",
          text: "A program you name so others can reach it: `tmt run reviewer claude`, or `tmt name reviewer` from inside one.",
        },
        {
          name: "request",
          text: 'What one agent sends another: `tmt talk builder "…"` stores it with an ID and a receipt and delivers it.',
        },
        {
          name: "result",
          text: "What comes back: the agent submits it with `tmt reply`, and `tmt result req_…` reads it.",
        },
      ],
      receipt: "receipt",
      receiptText:
        "The receipt ties a result to its request: a reply must carry the one its request came with.",
    },
    // The home page's tour: one section per layer, each with a short claim,
    // the chapter's own scene and a link to the chapter.
    showcase: {
      chapter: "Read the chapter →",
      sections: {
        start: {
          eyebrow: "start",
        },
        working: {
          eyebrow: "working",
          title: "A request *travels* pane to pane.",
          text: "`tmt talk` types your request into the other agent's pane, and its reply comes back to yours, matched by its receipt. You never copy text between windows.",
        },
        squad: {
          eyebrow: "squad",
          title: "Every mark means *one thing*.",
          text: "The Squad extension puts a lead and its members on one board, in the terminal you already use.",
        },
        colab: {
          eyebrow: "colab",
          title: "One page your whole *team* shares.",
          text: "The lead's plan, notes and decisions on one page that agents on any machine and your teammates read and comment on.",
        },
        meet: {
          eyebrow: "meet",
          title: "A room for you and your *agents*.",
          text: "One meeting for you, the lead and its members. Whoever wants to speak raises a hand, and you give the floor. Text first.",
        },
        install: {
          eyebrow: "install",
          title: "Install, then *name* your first agent.",
        },
      },
    },
  },
  chapters: {
    // The working chapter's opening scene: a request travelling pane to pane.
    travel: {
      label:
        "A request travels from lead to builder and the reply comes back. Lead runs tmt talk. tmt stores the request with an ID and a receipt. tmt types it into builder's pane. Builder works, then answers with tmt reply and the receipt. tmt stores the reply and talk hands it back to lead.",
      lead: "lead",
      builder: "builder",
      tmtPane: "tmt",
      exchange: "exchange",
      examples:
        "Any agent in any harness fits. Claude Code and Codex are two examples, and tmux is the built-in host today.",
      steps: [
        {
          title: "1 · talk",
          text: "lead sends a request. tmt stores it with an ID and a receipt.",
        },
        {
          title: "2 · deliver",
          text: "The request is typed into builder's pane. It works as usual.",
        },
        {
          title: "3 · reply",
          text: "builder answers with the receipt. The reply is stored, and talk hands it to lead.",
        },
      ],
    },
    // The colab chapter's opening scene: a shared page, a decision, comments.
    colab: {
      label:
        "A sketch of a shared colab page: a release plan whose items get checked off, a decision waiting on you, and comments from agents and a teammate.",
      pageTitle: "colab · release plan",
      sketch: "design sketch",
      planHeading: "Release 5.0",
      items: ["rotate tokens", "login tests", "docs for 5.0", "ship #412"],
      working: "working",
      needsYou: "needs you:",
      question: "ship #412 tonight?",
      shipTonight: "ship tonight",
      waitForDocs: "wait for docs",
      decided: "decided:",
      everyone: "everyone sees it",
      you: "you",
      decision: "decision",
      comments: [
        "Tokens rotated, PR #412 open.",
        "E2E green on the build server.",
        "Can we wait for the docs fix?",
        "Docs for 5.0 drafted, in review.",
      ],
      machines: ["your laptop", "build server", "Mei's laptop"],
      human: "human",
      roadmap: [
        { title: "now · local", text: "a page on your own machine (in progress)" },
        {
          title: "next · Firestore",
          text: "your Firebase project, so teammates can join (planned)",
        },
        { title: "later · Cloudflare", text: "your own Cloudflare account (planned)" },
      ],
    },
    // The meet chapter's opening scene: a text meeting where you give the floor.
    meet: {
      label:
        "A sketch of a text meeting in the terminal: builder has the floor, two agents have raised a hand, and you choose who speaks next.",
      title: "tmt meet · release window",
      sketch: "design sketch",
      floor: "floor",
      host: "you choose",
      first: "first come",
      hasFloor: "has the floor",
      hands: "hands",
      noHands: "no hands",
      inRoom: "in the room",
      keys: "g grant  @name grant  m mode",
      script: [
        {
          who: "builder",
          say: "Tokens are rotated. Tests pass. I need a decision on the release window.",
          hands: ["reviewer", "tester"],
        },
        {
          who: "reviewer",
          say: "I'd ship tonight: the diff is small and #412 is reviewed.",
          hands: ["tester"],
        },
        { who: "tester", say: "E2E is green on #412. No objection.", hands: [] },
        { who: "you", say: "Ship tonight. builder, merge after CI.", hands: ["builder"] },
      ],
      roadmap: [
        { title: "now · planned", text: "design agreed (#842), not started" },
        {
          title: "next · text",
          text: "text meetings in the terminal: raise a hand, take turns, you host",
        },
        { title: "later · web and voice", text: "a web view on colab, then voice" },
      ],
    },
    // The squad chapter's opening scene: what each board mark means.
    marks: {
      label: "The marks the board and every command use, each with one meaning.",
      intro:
        "The board stays quiet so the one thing that needs you stands out. The same marks appear in every command.",
      boardOnly: "board only",
      items: [
        { mark: "●", name: "running", text: "running or active" },
        { mark: "○", name: "offline", text: "offline or ended" },
        { mark: "◌", name: "no agent", text: "bound to a pane, no agent running" },
        { mark: "◆", name: "waits on you", text: "waits on your decision" },
        { mark: "✗", name: "blocked", text: "failed or blocked" },
        { mark: "✓", name: "done", text: "done" },
        { mark: "↻", name: "resume", text: "leads a resume action, never a row's state" },
        {
          mark: "▸",
          name: "folded",
          text: "a folded pane; unfold it with d or a click on its title",
        },
      ],
    },
  },
  journey: {
    stepsLabel: "Steps",
    layersLabel: "Layers on one foundation",
    // One entry per step: talk, board, colab, meet.
    steps: [
      {
        title: "1 · talk",
        hint: "Two panes, one message",
        status: "",
        caption:
          "The smallest start: two agents in two panes, and one sends the other a request with `tmt talk`. The reply comes back with a receipt, and everything below builds on exactly this.",
      },
      {
        title: "2 · board",
        hint: "The whole team in one view",
        status: "alpha",
        caption:
          "More agents? `tmt sq board` shows the whole team in one view: who works, who waits on you, who is blocked. It comes with the Squad extension. The same agents, one more window.",
      },
      {
        title: "3 · colab",
        hint: "One shared page",
        status: "in progress",
        caption:
          "Colab is a shared page for the lead's plan, notes and discussion, so teammates and agents on other machines read and comment on the same page. What you see is the design, not a release.",
      },
      {
        title: "4 · meet",
        hint: "Meet with your agents",
        status: "planned",
        caption:
          "Meet (#842) puts you, the lead and its members in one text meeting. Whoever wants to speak raises a hand, and you give the floor. Not started.",
      },
    ],
    // What each layer is, from meet down to the foundation.
    layers: [
      "a meeting room · planned",
      "a shared page · in progress",
      "the team in one view · alpha",
      "the foundation every layer uses",
    ],
    scenes: {
      talkArrow: "lead ⇄ builder · two panes · request out, reply back with a receipt",
      boardArrow: "same agents, one more window",
      boardNew: "new",
      yourLaptop: "your laptop",
      buildServer: "build server",
      teammate: "teammate",
      human: "(human)",
      herLead: "her lead",
      sharedPage: "one shared page",
      inProgress: "in progress",
      pageTitle: "colab · release plan",
      planHeading: "Release 5.0 plan",
      planOne: "rotate tokens",
      planTwo: "login tests",
      planThree: "ship #412 tonight",
      decide: "decide",
      reviewerSays: "diff is small, ok to ship",
      meiSays: "can we wait for the docs fix?",
      meetArrow: "raised hands (↑) wait their turn · you give the floor",
      planned: "planned",
    },
  },
};

type Widen<T> = T extends string
  ? string
  : T extends readonly (infer U)[]
    ? Widen<U>[]
    : { [K in keyof T]: Widen<T[K]> };

export type Strings = Widen<typeof english>;

export type StringsOverride<T = Strings> = T extends string
  ? string
  : T extends (infer U)[]
    ? StringsOverride<U>[]
    : { [K in keyof T]?: StringsOverride<T[K]> };

// A strings.json also carries a reserved top-level "$source" object ({ source,
// sourceRevision }) for the translation staleness check. It is not a string key:
// overlay walks the English keys only, so it is never merged.
// Overlays a language's strings on the English ones. The JSON is not typed by
// the compiler, so only a value of the same kind (text, list or group) as the
// English one is taken; anything else, and any unknown key, is ignored.
export function overlay<T>(base: T, over: unknown): T {
  if (typeof base === "string") return (typeof over === "string" && over ? over : base) as T;
  if (Array.isArray(base)) {
    const list = Array.isArray(over) ? over : [];
    return base.map((item, index) => overlay(item, list[index])) as T;
  }
  if (base && typeof base === "object") {
    const group = over && typeof over === "object" ? (over as Record<string, unknown>) : {};
    return Object.fromEntries(
      Object.entries(base).map(([key, value]) => [key, overlay(value, group[key])]),
    ) as T;
  }
  return base;
}
