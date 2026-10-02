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
  home: {
    eyebrow: "home",
    title: "One simple core. *Endless* ways for AI to work together.",
    lede: "tmt passes messages between agents and keeps a receipt for every reply. The board, colab and meet are built on that one core. It is made for any agent, any harness and any machine; the row below shows what runs today and what is on the way.",
    worksWith: "Works with",
    yourHarness: "your harness",
    yourServer: "your server",
    planned: "planned",
    designing: "designing",
    install: "Install",
    start: "Start with one message ↓",
    boardLabel:
      "A tmt sq board that updates in four steps: builder works on rotating tokens, reviewer starts a review and then waits on you to decide whether to ship, tester starts end-to-end tests, and once you answer the others carry on.",
    boardWaits: "◆ reviewer waits on you",
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
      youHost: "you · host",
      speaking: "speaking",
      meetSays: "Tests pass on both machines. Who decides the release window?",
      meetArrow: "raised hands wait their turn · you give the floor",
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
