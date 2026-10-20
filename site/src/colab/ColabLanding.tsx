import { useAtom } from "jotai";
import { useState } from "react";
import {
  ArrowDown,
  ArrowRight,
  Bot,
  Check,
  ChevronRight,
  Circle,
  FileText,
  Folder,
  MousePointer2,
  Moon,
  Sun,
  UserRound,
} from "lucide-react";
import { themeAtom } from "../state/theme";
import "./colab-landing.css";

const features = [
  { id: "annotate", number: "01", label: "Annotate" },
  { id: "agents", number: "02", label: "Talk to agents" },
  { id: "collaborate", number: "03", label: "Write together" },
  { id: "organize", number: "04", label: "Organize work" },
] as const;

const backendGuides = {
  local: {
    label: "Local",
    status: "Available",
    commandLabel: "RUN ON YOUR MACHINE",
    command: "tmt colab serve",
    note: "Open a local page and pair a new browser when prompted.",
    capabilities: ["Available", "Title, label, author", "Not yet", "Not yet"],
  },
  firestore: {
    label: "Firestore",
    status: "Hosted entry in progress",
    commandLabel: "READ-ONLY DEPLOYMENT PLAN",
    command:
      "tmt remote deploy firestore --project YOUR_PROJECT --region YOUR_REGION --sign-in anonymous --json",
    note: "Inspect the exact plan first. When hosted Colab is ready, review it, rerun with --authorize PLAN_DIGEST_PREFIX, then pair each browser.",
    capabilities: ["In progress", "In progress", "Not yet", "Not yet"],
  },
  cloudflare: {
    label: "Cloudflare",
    status: "Planned",
    commandLabel: "DEPLOYMENT",
    command: null,
    note: "The Cloudflare backend and its deployment command are planned, not available yet.",
    capabilities: ["Planned", "Planned", "Not yet", "Not yet"],
  },
  own: {
    label: "Your own provider",
    status: "Developer path",
    commandLabel: "START SELF-HOSTED",
    command: "tmt colab serve",
    note: "One command runs your page locally. A custom Remote backend needs an adapter and conformance checks; hosted setup is still a developer path.",
    capabilities: ["Not wired yet", "Not wired yet", "Not yet", "Not yet"],
  },
} as const;

function DemoHeader({ title, end }: { title: string; end?: string }) {
  return (
    <div className="cl-demo-header">
      <span className="cl-demo-icon">
        <FileText size={16} />
      </span>
      <strong>{title}</strong>
      {end && <span className="cl-demo-header-end">{end}</span>}
    </div>
  );
}

function Mention({ name, kind }: { name: string; kind: "agent" | "person" }) {
  const Icon = kind === "agent" ? Bot : UserRound;
  return (
    <span className={`cl-mention cl-mention-${kind}`} aria-label={`@${name}, ${kind}`}>
      <Icon size={12} aria-hidden="true" />@{name}
    </span>
  );
}

function MentionPicker({ selected }: { selected: "agent" | "person" }) {
  return (
    <span className="cl-compose-autocomplete" aria-hidden="true">
      <span className={`cl-picker-option ${selected === "agent" ? "cl-picker-selected" : ""}`}>
        <Bot size={12} /> <b>reviewer</b> <small>agent</small>
      </span>
      <span className={`cl-picker-option ${selected === "person" ? "cl-picker-selected" : ""}`}>
        <UserRound size={12} /> <b>Ben</b> <small>person</small>
      </span>
    </span>
  );
}

function SelectedSentence({ variant }: { variant: "annotation" | "agent" }) {
  return (
    <p className={`cl-demo-selection cl-demo-selection-${variant}`}>
      Check the mobile layout before release.
      <span className="cl-selection-drag-caret" aria-hidden="true" />
      <span className="cl-selection-pin" aria-hidden="true">
        +
      </span>
      {variant === "annotation" && (
        <MousePointer2 className="cl-demo-pointer" size={18} aria-hidden="true" />
      )}
    </p>
  );
}

function TypedRun({
  phase,
  children,
  label,
}: {
  phase: "agent" | "person" | "collab";
  children: string;
  label?: string;
}) {
  return (
    <span className={`cl-typed-run cl-typed-run-${phase}`}>
      <span className="cl-typed-text">{children}</span>
      <span className="cl-typed-caret" aria-hidden="true">
        {label && <span>{label}</span>}
      </span>
    </span>
  );
}

function AnnotationVisual() {
  return (
    <div
      className="cl-visual cl-annotation-visual"
      aria-label="A selected sentence with an annotation being composed beside it"
    >
      <DemoHeader title="Launch brief" end="Private · Live" />
      <div className="cl-annotation-body">
        <div className="cl-sample-document">
          <p className="cl-overline">PROJECT / LAUNCH</p>
          <h3>
            A smaller release.
            <br />A clearer first step.
          </h3>
          <p>The plan, the conversation, and the work stay in one place.</p>
          <div className="cl-sample-rule" />
          <p className="cl-overline">01 / RELEASE CHECKLIST</p>
          <div className="cl-selection-anchor">
            <SelectedSentence variant="annotation" />
            <div className="cl-annotation-composer">
              <span className="cl-compose-caption">ANNOTATE THE SELECTION</span>
              <p className="cl-compose-draft">
                <span className="cl-compose-line cl-compose-line-agent">
                  <span className="cl-compose-query" aria-hidden="true">
                    @
                  </span>
                  <span className="cl-compose-mention">
                    <Mention name="reviewer" kind="agent" />
                  </span>
                  <TypedRun phase="agent"> check 390px.</TypedRun>
                  <MentionPicker selected="agent" />
                </span>
                <span className="cl-compose-line cl-compose-line-person">
                  <span className="cl-compose-query" aria-hidden="true">
                    @
                  </span>
                  <span className="cl-compose-mention">
                    <Mention name="Ben" kind="person" />
                  </span>
                  <TypedRun phase="person"> review copy?</TypedRun>
                  <MentionPicker selected="person" />
                </span>
              </p>
              <div className="cl-annotation-compose-footer">
                <span>Attached to this sentence</span>
                <span className="cl-compose-send" aria-label="Send annotation preview">
                  <ArrowRight size={16} />
                </span>
              </div>
            </div>
          </div>
          <p>Keep the preview focused.</p>
          <p>Share the decision with the team.</p>
        </div>
      </div>
    </div>
  );
}

function AgentsVisual() {
  return (
    <div
      className="cl-visual cl-agents-visual"
      aria-label="Illustrated sequence: select text, mention an agent and a teammate, then receive both replies"
    >
      <DemoHeader title="Launch brief" end="Live discussion" />
      <div className="cl-agent-demo-grid">
        <div className="cl-agent-document">
          <p className="cl-overline">SHARED DOCUMENT</p>
          <h3>Launch brief</h3>
          <p>One place to plan the release together.</p>
          <div className="cl-sample-rule" />
          <p className="cl-overline">01 / RELEASE CHECKLIST</p>
          <SelectedSentence variant="agent" />
          <p className="cl-agent-document-note">The discussion starts on this sentence.</p>
        </div>
        <div className="cl-agent-thread">
          <div className="cl-agent-thread-heading">
            <p className="cl-overline">ANNOTATION / 01</p>
            <span>2 people · 1 agent</span>
          </div>
          <div className="cl-scene-message cl-scene-user">
            <div className="cl-scene-author">
              <span className="cl-scene-avatar">Y</span>
              <b>You</b>
              <small>annotated this line</small>
            </div>
            <p>
              <Mention name="reviewer" kind="agent" /> check 390px.{" "}
              <Mention name="Ben" kind="person" />
              review the copy?
            </p>
          </div>
          <div className="cl-scene-message cl-scene-agent">
            <div className="cl-scene-author">
              <span className="cl-scene-avatar cl-scene-avatar-agent">
                <Bot size={13} />
              </span>
              <b>reviewer</b>
              <small>agent</small>
            </div>
            <p>The layout fits at 390px. I left a note on the selection.</p>
          </div>
          <div className="cl-scene-message cl-scene-person">
            <div className="cl-scene-author">
              <span className="cl-scene-avatar cl-scene-avatar-person">
                <UserRound size={13} />
              </span>
              <b>Ben</b>
              <small>teammate</small>
            </div>
            <p>Copy looks good. Let’s keep the first step focused.</p>
          </div>
        </div>
      </div>
    </div>
  );
}

function CollaborationVisual() {
  return (
    <div
      className="cl-visual cl-collaboration-visual"
      aria-label="Illustrated sequence: Ben reviews a task, then Maya edits the shared document"
    >
      <DemoHeader title="Release plan" end="Live document" />
      <div className="cl-collaboration-body">
        <div className="cl-editor-sample">
          <div className="cl-presence-line">
            <span className="cl-live-dot" /> Live on this page
            <span className="cl-avatar cl-collab-avatar-ben" aria-label="Ben is here">
              B
            </span>
            <span
              className="cl-avatar cl-avatar-alt cl-collab-avatar-maya"
              aria-label="Maya is here"
            >
              M
            </span>
          </div>
          <p className="cl-overline">STUDIO / Q4</p>
          <h3>Release plan</h3>
          <p>One document everyone can return to.</p>
          <div className="cl-sample-rule" />
          <p className="cl-collab-review-line">
            <Check size={16} /> Review the scope <span className="cl-collab-review-by">Ben</span>
          </p>
          <p>
            <Check size={16} /> Agree on the first cut
          </p>
          <p className="cl-editing-line" aria-label="Maya is editing">
            <Circle size={16} />
            <TypedRun phase="collab" label="Maya">
              Polish the mobile page
            </TypedRun>
          </p>
        </div>
        <div className="cl-activity-sample">
          <p className="cl-overline">IN THE DOCUMENT</p>
          <div className="cl-activity-entry cl-activity-ben">
            <span className="cl-activity-avatar">B</span>
            <div>
              <strong>Ben reviewed the scope</strong>
              <span>Ready for the first cut.</span>
              <small>Just now</small>
            </div>
          </div>
          <div className="cl-activity-entry cl-activity-maya">
            <span className="cl-activity-avatar cl-activity-avatar-maya">M</span>
            <div>
              <strong>Maya updated the plan</strong>
              <span>The change appears here for everyone with access.</span>
              <small>Live edit</small>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function OrganizationVisual() {
  return (
    <div
      className="cl-visual cl-organization-visual"
      aria-label="Pages and a project plan with checklist and decision"
    >
      <DemoHeader title="Studio pages" end="Pages" />
      <div className="cl-organization-body">
        <aside className="cl-page-list">
          <p className="cl-overline">PAGES</p>
          <div className="cl-page-row cl-page-row-active">
            <FileText size={16} />
            <span>Launch plan</span>
            <ChevronRight size={15} />
          </div>
          <div className="cl-page-row">
            <FileText size={16} />
            <span>Design review</span>
          </div>
          <div className="cl-page-row">
            <FileText size={16} />
            <span>Decision log</span>
          </div>
          <div className="cl-labels">
            <Folder size={15} /> design <span>3 pages</span>
          </div>
        </aside>
        <div className="cl-plan-sample">
          <p className="cl-overline">PROJECT / LAUNCH</p>
          <h3>Launch plan</h3>
          <p>Keep the brief, decisions, and next steps together.</p>
          <div className="cl-sample-rule" />
          <p className="cl-overline">NEXT STEPS</p>
          <div className="cl-plan-row">
            <Check size={16} /> Agree on scope <span>Done</span>
          </div>
          <div className="cl-plan-row">
            <Circle size={16} /> Check the mobile layout <span>In review</span>
          </div>
          <div className="cl-plan-row">
            <Circle size={16} /> Share the release note <span>Next</span>
          </div>
          <div className="cl-decision-sample">
            <b>Decision</b>
            <span>Ship the focused first cut.</span>
          </div>
        </div>
      </div>
    </div>
  );
}

const install = [
  {
    label: "Install TMT",
    command: "curl -fsSL https://github.com/pj-tmt/tmt/releases/latest/download/install.sh | sh",
  },
  { label: "Add Colab", command: "tmt extension install colab --skills" },
  { label: "Open Colab", command: "tmt colab serve" },
  {
    label: "Create a page",
    command: 'tmt colab page create --title "Launch plan" --layout column',
  },
];

export function ColabLanding() {
  const [backend, setBackend] = useState<keyof typeof backendGuides>("local");
  const selectedBackend = backendGuides[backend];
  const [theme, setTheme] = useAtom(themeAtom);
  return (
    <div className="colab-landing">
      <header className="cl-header">
        <div className="cl-brand" aria-label="TMT Colab">
          <a href={import.meta.env.BASE_URL} className="cl-brand-parent" aria-label="TMT home">
            <span className="cl-logo" aria-hidden="true" />
            <span>TMT</span>
          </a>
          <span className="cl-brand-divider" aria-hidden="true" />
          <span className="cl-brand-product">Colab</span>
        </div>
        <nav className="cl-nav" aria-label="Colab features">
          {features.map((feature) => (
            <a key={feature.id} href={`#${feature.id}`}>
              <span>{feature.number}</span> {feature.label}
            </a>
          ))}
        </nav>
        <div className="cl-header-actions">
          <a className="cl-header-cta" href="#install">
            Get started <ArrowRight size={15} />
          </a>
          <button
            type="button"
            aria-label="Change theme"
            onClick={() => setTheme(theme === "light" ? "dark" : "light")}
          >
            {theme === "light" ? <Moon size={18} /> : <Sun size={18} />}
          </button>
        </div>
      </header>

      <main>
        <section className="cl-hero" aria-labelledby="cl-hero-title">
          <div className="cl-hero-copy">
            <p className="cl-overline">SELF-HOSTED PAGES FOR PEOPLE AND AGENTS</p>
            <h1 id="cl-hero-title">
              Annotate the work.
              <br />
              Bring your agents in.
            </h1>
            <p>
              <span className="cl-hero-highlight">Select a point.</span> On a page you host, mention
              TMT-connected agents and teammates; their replies stay with the work.
            </p>
            <div className="cl-hero-actions">
              <a className="cl-primary-link" href="#annotate">
                Explore Colab <ArrowDown size={17} />
              </a>
              <a className="cl-secondary-link" href="#install">
                Install & use <ArrowRight size={16} />
              </a>
            </div>
          </div>
          <div className="cl-hero-visual">
            <AnnotationVisual />
          </div>
        </section>

        <nav className="cl-feature-index" aria-label="Feature close-ups">
          {features.map((feature) => (
            <a key={feature.id} href={`#${feature.id}`}>
              <span>{feature.number}</span>
              <b>{feature.label}</b>
              <ArrowRight size={16} />
            </a>
          ))}
        </nav>

        <section className="cl-feature" id="annotate" aria-labelledby="cl-annotate-title">
          <div className="cl-feature-copy">
            <span className="cl-feature-number">01 / 04</span>
            <h2 id="cl-annotate-title">Start with a precise point.</h2>
            <p>
              <mark className="cl-copy-highlight">Select a sentence.</mark> Add an annotation and
              keep the discussion attached to the work. The answer comes back to that same place.
            </p>
            <a href="#agents">
              Next: talk to agents <ArrowRight size={16} />
            </a>
          </div>
          <AnnotationVisual />
        </section>
        <section className="cl-feature" id="agents" aria-labelledby="cl-agents-title">
          <div className="cl-feature-copy">
            <span className="cl-feature-number">02 / 04</span>
            <h2 id="cl-agents-title">
              Talk to agents from the page.
              <span className="cl-harness-heading">With your TMT-connected agents.</span>
            </h2>
            <p>
              <mark className="cl-copy-highlight">Mention an agent and a teammate</mark> in one
              annotation. With Remote, reach agents in another environment too.
            </p>
            <a href="#collaborate">
              Next: write together <ArrowRight size={16} />
            </a>
          </div>
          <AgentsVisual />
        </section>
        <section className="cl-feature" id="collaborate" aria-labelledby="cl-collaborate-title">
          <div className="cl-feature-copy">
            <span className="cl-feature-number">03 / 04</span>
            <h2 id="cl-collaborate-title">Write together, live.</h2>
            <p>
              <mark className="cl-copy-highlight">The document updates as people work.</mark> Notes,
              replies, and the latest version stay with the page.
            </p>
            <a href="#organize">
              Next: organize work <ArrowRight size={16} />
            </a>
          </div>
          <CollaborationVisual />
        </section>
        <section className="cl-feature" id="organize" aria-labelledby="cl-organize-title">
          <div className="cl-feature-copy">
            <span className="cl-feature-number">04 / 04</span>
            <h2 id="cl-organize-title">Give the project a place.</h2>
            <p>
              <mark className="cl-copy-highlight">Collect plans, checklists, and decisions.</mark>{" "}
              Use labels to find related pages again.
            </p>
            <a href="#install">
              Next: get started <ArrowRight size={16} />
            </a>
          </div>
          <OrganizationVisual />
        </section>

        <section className="cl-install" id="install" aria-labelledby="cl-install-title">
          <div className="cl-install-intro">
            <span className="cl-feature-number">GET STARTED</span>
            <h2 id="cl-install-title">One page is enough to begin.</h2>
            <p>
              Install TMT and Colab. <mark className="cl-copy-highlight">Start a local page</mark>,
              then pair each new browser when prompted.
            </p>
          </div>
          <ol className="cl-install-steps">
            {install.map((step, index) => (
              <li key={step.label}>
                <span className="cl-install-number">{String(index + 1).padStart(2, "0")}</span>
                <div>
                  <h3>{step.label}</h3>
                  <code>{step.command}</code>
                </div>
              </li>
            ))}
          </ol>
        </section>
        <section className="cl-deploy" id="deploy" aria-labelledby="cl-deploy-title">
          <div className="cl-deploy-intro">
            <span className="cl-feature-number">HOST IT YOUR WAY</span>
            <h2 id="cl-deploy-title">Start local. Plan for your team.</h2>
            <p>
              <mark className="cl-copy-highlight">Host the page yourself.</mark> Start locally, then
              explore the Remote options below.
            </p>
          </div>
          <div className="cl-deploy-details">
            <div className="cl-backend-tabs" role="tablist" aria-label="Colab backend">
              {(Object.keys(backendGuides) as (keyof typeof backendGuides)[]).map((key) => (
                <button
                  key={key}
                  type="button"
                  role="tab"
                  id={"cl-backend-tab-" + key}
                  aria-controls="cl-backend-panel"
                  aria-selected={backend === key}
                  onClick={() => setBackend(key)}
                >
                  {backendGuides[key].label}
                </button>
              ))}
            </div>
            <div
              className="cl-backend-panel"
              role="tabpanel"
              id="cl-backend-panel"
              aria-labelledby={"cl-backend-tab-" + backend}
            >
              <div className="cl-deploy-command">
                <span className="cl-overline">{selectedBackend.commandLabel}</span>
                {selectedBackend.command ? (
                  <code>{selectedBackend.command}</code>
                ) : (
                  <p className="cl-deploy-unavailable">No command available yet.</p>
                )}
                <p>{selectedBackend.note}</p>
                {backend === "own" && (
                  <p>
                    Read the{" "}
                    <a href="https://github.com/pj-tmt/tmt/tree/main/.agents/skills/tmt-remote/references">
                      Remote backend references
                    </a>{" "}
                    for the adapter and conformance guides, and the{" "}
                    <a href="https://github.com/pj-tmt/tmt/blob/main/extensions/tmt-colab/skills/tmt-colab/SKILL.md">
                      Colab agent skill
                    </a>{" "}
                    for page workflows.
                  </p>
                )}
              </div>
              <table className="cl-capability-table">
                <caption>
                  {selectedBackend.label} · {selectedBackend.status}
                </caption>
                <thead>
                  <tr>
                    <th scope="col">Capability</th>
                    <th scope="col">Colab status</th>
                  </tr>
                </thead>
                <tbody>
                  {["Realtime editing", "Page search", "Full-text search", "Vector search"].map(
                    (capability, index) => (
                      <tr key={capability}>
                        <th scope="row">{capability}</th>
                        <td>{selectedBackend.capabilities[index]}</td>
                      </tr>
                    ),
                  )}
                </tbody>
              </table>
            </div>
            <p className="cl-deploy-note">
              Status here describes Colab features, not everything the underlying providers can
              offer. Realtime editing means shared document updates; full-text and vector search are
              not Colab features yet.
            </p>
          </div>
        </section>
      </main>
      <footer className="cl-footer">
        <a href={import.meta.env.BASE_URL} className="cl-footer-brand">
          <span className="cl-logo" aria-hidden="true" /> Colab
        </a>
        <span>Pages for people and agents.</span>
        <a href="https://github.com/pj-tmt/tmt">
          GitHub <ArrowRight size={15} />
        </a>
      </footer>
    </div>
  );
}
