import { Component, Fragment, type ReactNode } from 'react';
import { BrowserAction, HostedShell } from '@tmt/browser-ui/react';
import type { ShellSlot } from '@tmt/browser-ui/shell-host';
import { ColabHeader } from './colab-header.js';
import { ErrorDetails, NoticeCard } from './notice-card.js';
import { text } from './strings.js';

/**
 * What the TMT web app shows when Colab crashes above its router (#4373). The host's rail and
 * route stay; Colab's error view fills the module slot and its title goes to the host's header
 * slot, with no router, session or list to read: this view has to work when nothing else does.
 */
function CrashView({ slot, error, retry }: { slot: ShellSlot; error: unknown; retry(): void }) {
  return (
    <HostedShell slot={slot} menu={undefined}>
      <ColabHeader title={text.crashTitle} remoteMenu={false} remoteChip={false} />
      <main>
        <NoticeCard
          state="blocked"
          eyebrow={text.product}
          title={text.crashTitle}
          actions={
            <BrowserAction
              type="button"
              variant="text"
              label={text.crashRetry}
              onActivate={retry}
            />
          }
        >
          <p>{text.crashBody}</p>
          <ErrorDetails error={error} />
        </NoticeCard>
      </main>
    </HostedShell>
  );
}

interface State {
  failed: boolean;
  error?: unknown;
  /** Changes on Try again, so everything below mounts fresh: a new session and a new router. */
  attempt: number;
}

/**
 * The one error boundary above everything Colab renders inside the TMT web app. A route error is
 * still the router's own `RootError`; this catches what throws outside it (the mounted app, the
 * router provider, the update row). Leaving the route unmounts the whole root, so a failed state
 * never outlives the route: coming back builds a new root.
 */
export class ColabModuleBoundary extends Component<
  { slot: ShellSlot; children: ReactNode },
  State
> {
  state: State = { failed: false, attempt: 0 };
  static getDerivedStateFromError(error: unknown): Partial<State> {
    return { failed: true, error };
  }
  componentDidCatch(error: unknown) {
    console.error(error);
  }
  render() {
    if (this.state.failed)
      return (
        <CrashView
          slot={this.props.slot}
          error={this.state.error}
          retry={() => this.setState((state) => ({ failed: false, attempt: state.attempt + 1 }))}
        />
      );
    return <Fragment key={this.state.attempt}>{this.props.children}</Fragment>;
  }
}
