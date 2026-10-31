import { useState, type ReactNode } from 'react';
import { BrandMark } from '../src/brand-mark.js';
import { UiTheme } from '../src/ui/index.js';
import { components, type ComponentDoc } from './catalog.js';
import { patterns } from './patterns.js';
import { tokenPages } from './tokens.js';
import { radiusLabel, setRadius, setTheme, usePreview, type Radius } from './preview.js';
import { follow } from './router.js';

interface Page {
  href: string;
  title: string;
  render(): ReactNode;
}
const intro: Page = { href: '/docs', title: 'Introduction', render: () => <Introduction /> };
const principles: Page = {
  href: '/docs/principles',
  title: 'Principles',
  render: () => <Principles />,
};
const sections: { title: string; pages: Page[] }[] = [
  { title: 'Getting started', pages: [intro, principles] },
  {
    title: 'Tokens',
    pages: tokenPages.map((page) => ({
      href: `/docs/tokens/${page.slug}`,
      title: page.title,
      render: () => (
        <Article title={page.title} lead={page.lead}>
          {page.render()}
        </Article>
      ),
    })),
  },
  {
    title: 'Components',
    pages: [...components]
      .sort((a, b) => a.name.localeCompare(b.name, 'en'))
      .map((doc) => ({
        href: `/docs/components/${doc.slug}`,
        title: doc.name,
        render: () => <ComponentPage doc={doc} />,
      })),
  },
  {
    title: 'Patterns',
    pages: patterns.map((pattern) => ({
      href: `/docs/patterns/${pattern.slug}`,
      title: pattern.title,
      render: () => (
        <Article title={pattern.title} lead={pattern.lead}>
          {pattern.render()}
        </Article>
      ),
    })),
  },
];
const pages = sections.flatMap((section) => section.pages);

export function DocsApp({ path }: { path: string }) {
  const { theme } = usePreview();
  const page = pages.find((entry) => entry.href === path) ?? intro;
  const index = pages.indexOf(page);
  const [menuOpen, setMenuOpen] = useState(false);
  return (
    <UiTheme theme={theme}>
      <div className="docs">
        <TopBar onMenu={() => setMenuOpen((open) => !open)} />
        <div className="docs-body">
          <nav className="docs-nav" data-open={menuOpen || undefined} aria-label="Documentation">
            {sections.map((section) => (
              <div key={section.title} className="docs-nav-group">
                <p className="docs-nav-title">{section.title}</p>
                {section.pages.map((entry) => (
                  <a
                    key={entry.href}
                    href={entry.href}
                    aria-current={entry === page ? 'page' : undefined}
                    onClick={(event) => {
                      setMenuOpen(false);
                      follow(event);
                    }}
                  >
                    {entry.title}
                  </a>
                ))}
              </div>
            ))}
          </nav>
          <main className="docs-main" key={page.href}>
            {page.render()}
            <footer className="docs-pager">
              {index > 0 ? (
                <a href={pages[index - 1]!.href} onClick={follow}>
                  <span>Previous</span>
                  {pages[index - 1]!.title}
                </a>
              ) : (
                <span />
              )}
              {index < pages.length - 1 && (
                <a href={pages[index + 1]!.href} onClick={follow} data-next>
                  <span>Next</span>
                  {pages[index + 1]!.title}
                </a>
              )}
            </footer>
          </main>
        </div>
      </div>
    </UiTheme>
  );
}

export function TopBar({ onMenu }: { onMenu?: () => void }) {
  const { theme, radius } = usePreview();
  return (
    <header className="docs-top">
      {onMenu && (
        <button type="button" className="docs-menu-button" aria-label="Menu" onClick={onMenu}>
          ☰
        </button>
      )}
      <a className="docs-brand" href="/docs" onClick={follow}>
        <BrandMark size={22} />
        <span>tmt</span>
        <span className="docs-brand-sub">design system</span>
      </a>
      <nav className="docs-top-links" aria-label="Sections">
        <a href="/docs" onClick={follow}>
          Docs
        </a>
        <a href="/docs/components/button" onClick={follow}>
          Components
        </a>
        <a href="/mock/colab" onClick={follow}>
          Colab mock
        </a>
      </nav>
      <div className="docs-switches">
        <div className="docs-segment" role="group" aria-label="Radius">
          {(['2', '4', 'current'] as Radius[]).map((value) => (
            <button
              key={value}
              type="button"
              aria-pressed={radius === value}
              onClick={() => setRadius(value)}
            >
              {radiusLabel[value]}
            </button>
          ))}
        </div>
        <button
          type="button"
          className="docs-theme"
          aria-label={theme === 'dark' ? 'Switch to light' : 'Switch to dark'}
          onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}
        >
          {theme === 'dark' ? '☾ Dark' : '☀ Light'}
        </button>
      </div>
    </header>
  );
}

export function Article({
  title,
  lead,
  children,
}: {
  title: string;
  lead: ReactNode;
  children: ReactNode;
}) {
  return (
    <article className="docs-article">
      <h1>{title}</h1>
      <p className="docs-lead">{lead}</p>
      {children}
    </article>
  );
}

function Introduction() {
  return (
    <Article
      title="TMT design system"
      lead="One family of components and tokens for Colab, Remote, Ops and the blog. What you see here is the code: every preview is the real component, skinned only by tokens."
    >
      <h2>How it is built</h2>
      <p>
        The tokens live in <code>design/tokens/tokens.json</code> in three tiers. <b>Ref</b> tokens
        are raw values. <b>Semantic roles</b> such as <code>--tmt-ui-color-accent</code> are what
        components read. <b>Component tokens</b> alias the roles inside the shadcn-based parts.
        Palette B, red, black, white and grey, maps the roles onto refs.
      </p>
      <p>
        The components come from <code>@tmt/browser-ui</code>. The <code>/ui</code> entry is the
        shadcn set on Radix; the <code>Browser*</code> family from <code>/react</code> moves onto
        those internals next, so there is one family.
      </p>
      <h2>Use the switches</h2>
      <p>
        The top bar switches the theme and the one radius knob for every page and the Colab mock:
        2/4, 4/6, or today&apos;s values for comparison.
      </p>
      <div className="docs-cards">
        <a className="docs-card" href="/docs/principles" onClick={follow}>
          <b>Principles</b>
          <span>Five rules every screen follows.</span>
        </a>
        <a className="docs-card" href="/docs/tokens/colour" onClick={follow}>
          <b>Tokens</b>
          <span>Colour, type, spacing, radius and motion, read live.</span>
        </a>
        <a className="docs-card" href="/docs/components/button" onClick={follow}>
          <b>Components</b>
          <span>{components.length} parts, each with variants, states and usage.</span>
        </a>
        <a className="docs-card" href="/mock/colab" onClick={follow}>
          <b>Colab mock</b>
          <span>The side menu, page list and page view rebuilt from these parts.</span>
        </a>
      </div>
    </Article>
  );
}

function Principles() {
  const rules: [string, string][] = [
    [
      'Red is rare',
      'The accent red marks only the brand, keyboard focus, the selected tab and the one primary action on a surface. Nothing else is red-filled.',
    ],
    [
      'Status is never the accent',
      'Working, waiting, review and blocked have their own muted hues and always a mark and a word (● ◆ ◐ ✕). Blocked is a darker red with its ✕, never the accent.',
    ],
    [
      'Corners are small',
      'Controls, fields and tags share one radius; cards and surfaces are one step larger. Only avatars and status dots are round.',
    ],
    [
      'Real components only',
      'A mock is built from the same components the product ships, so a mock is the implementation and review compares like with like.',
    ],
    [
      'One family',
      'One set of components and tokens serves Colab, Remote, Ops and the blog. Product code adds no colours, radii or local copies.',
    ],
  ];
  return (
    <Article title="Principles" lead="Short rules that decide most questions before they come up.">
      <ol className="docs-principles">
        {rules.map(([title, body]) => (
          <li key={title}>
            <h2>{title}</h2>
            <p>{body}</p>
          </li>
        ))}
      </ol>
    </Article>
  );
}

function ComponentPage({ doc }: { doc: ComponentDoc }) {
  return (
    <Article title={doc.name} lead={doc.description}>
      <div className="docs-canvas docs-canvas-hero">{doc.preview()}</div>
      <h2>Usage</h2>
      <Snippet code={doc.usage} />
      {doc.variants.length > 0 && <h2>Variants and states</h2>}
      {doc.variants.map(([label, render]) => (
        <section key={label} className="docs-variant">
          <h3>{label}</h3>
          <div className="docs-canvas">{render()}</div>
        </section>
      ))}
      {doc.dos && (
        <>
          <h2>Do and don&apos;t</h2>
          <div className="docs-dos">
            {doc.dos.map(([kind, text]) => (
              <div key={text} className="docs-do" data-kind={kind}>
                <b>{kind === 'do' ? 'Do' : 'Don’t'}</b>
                <p>{text}</p>
              </div>
            ))}
          </div>
        </>
      )}
    </Article>
  );
}

export function Snippet({ code }: { code: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="docs-snippet">
      <button
        type="button"
        onClick={() => {
          void navigator.clipboard?.writeText(code).then(() => {
            setCopied(true);
            setTimeout(() => setCopied(false), 1500);
          });
        }}
      >
        {copied ? 'Copied' : 'Copy'}
      </button>
      <pre>
        <code>{code}</code>
      </pre>
    </div>
  );
}
