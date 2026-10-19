import {
  Fragment,
  type MutableRefObject,
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react';
import {
  UiTheme,
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
} from '@tmt/browser-ui/ui';
import '@tmt/browser-ui/ui.css';
import { AgentList } from './components/agent-chip.js';
import type { NeedsYouCandidate } from './needs-you.js';
import { useNotificationModalPresence } from './reply-notification-context.js';
import { getTheme, subscribeTheme } from './theme.js';
import { text } from './strings.js';
import { openModifiedPage, plainPageClick } from './page-link.js';

/** Only disclosure/focus live here. Each page layer owns its pending state and navigation. */
export function NeedsYouHeader({
  items,
  href,
  onPick,
  menuFocus,
}: {
  items: readonly NeedsYouCandidate[];
  href(item: NeedsYouCandidate): string;
  onPick(item: NeedsYouCandidate): void;
  /** Owned by the host: whether an open menu held focus when this header went away. The header
   * remounts while its layers reparse, so the flag has to outlive one instance. */
  menuFocus: MutableRefObject<boolean>;
}) {
  const theme = useSyncExternalStore(subscribeTheme, getTheme, () => 'light' as const);
  const [open, setOpen] = useState(false);
  const pending = useRef<string | undefined>(undefined);
  const report = useNotificationModalPresence();
  useEffect(() => {
    if (!items.length) {
      setOpen(false);
      report(false);
    } else menuFocus.current = false;
  }, [items.length, report, menuFocus]);
  // The list emptied under an open menu: the menu and its trigger are gone and focus fell to the
  // body. The mark that replaces them takes it, again after a remount, until focus moves on.
  const nothingWaits = useCallback(
    (mark: HTMLSpanElement | null) => {
      if (!mark || !menuFocus.current) return;
      const active = mark.ownerDocument.activeElement;
      if (!active || active === mark.ownerDocument.body) mark.focus({ preventScroll: true });
    },
    [menuFocus],
  );
  function choose(item: NeedsYouCandidate) {
    pending.current = item.id;
    menuFocus.current = false;
    setOpen(false);
    report(false);
  }
  if (!items.length)
    return (
      <span
        ref={nothingWaits}
        tabIndex={-1}
        onBlur={() => {
          menuFocus.current = false;
        }}
        className="needs-you-header needs-you-header-empty"
        aria-label={text.needsYouNothing}
      >
        <span aria-hidden="true">
          ✓<span className="needs-you-wide"> {text.needsYouNothingWord}</span>
        </span>
      </span>
    );
  return (
    <span className="needs-you-header">
      <UiTheme theme={theme}>
        <DropdownMenu
          open={open}
          onOpenChange={(value) => {
            if (!value) menuFocus.current = false;
            setOpen(value);
            report(value);
          }}
        >
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="tmt-ui-action needs-you-header-trigger"
              data-variant="text"
              aria-label={text.needsYouLabel(items.length)}
              onPointerDownCapture={(event) => {
                if (!event.isTrusted) {
                  event.preventDefault();
                  event.stopPropagation();
                }
              }}
              onKeyDownCapture={(event) => {
                if (!event.isTrusted || event.nativeEvent.isComposing) {
                  event.preventDefault();
                  event.stopPropagation();
                }
              }}
            >
              <span className="needs-you-header-mark" aria-hidden="true">
                ◆
              </span>
              <span aria-hidden="true">
                <span className="needs-you-wide">{text.needsYouWord} </span>
                {items.length}
              </span>
              <span aria-hidden="true">▾</span>
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            className="needs-you-header-menu"
            align="end"
            onFocusCapture={() => {
              menuFocus.current = true;
            }}
            onCloseAutoFocus={(event) => {
              const id = pending.current;
              pending.current = undefined;
              const item = items.find((value) => value.id === id);
              if (!item) return;
              event.preventDefault();
              onPick(item);
            }}
            onClickCapture={(event) => {
              if (!event.isTrusted) {
                event.preventDefault();
                event.stopPropagation();
              }
            }}
            onKeyDownCapture={(event) => {
              if (!event.isTrusted || event.nativeEvent.isComposing) {
                event.preventDefault();
                event.stopPropagation();
              }
            }}
          >
            {(['proposal', 'reading', 'answer', 'review'] as const).map((kind) => {
              const rows = items.filter((item) => item.kind === kind);
              if (!rows.length) return null;
              const label = text.needsYouGroup(kind, rows.length);
              return (
                <Fragment key={kind}>
                  <DropdownMenuLabel className="needs-you-header-group">{label}</DropdownMenuLabel>
                  <DropdownMenuGroup aria-label={label}>
                    {rows.map((item) => (
                      <DropdownMenuItem
                        asChild
                        key={item.id}
                        data-needs-you-id={item.id}
                        data-needs-you-kind={item.kind}
                        onSelect={(event) => {
                          event.preventDefault();
                          choose(item);
                        }}
                      >
                        <a
                          href={href(item)}
                          onClickCapture={(event) => {
                            // Radix selection must not consume a modified anchor activation.
                            if (!plainPageClick(event)) event.stopPropagation();
                          }}
                          onClick={(event) => {
                            if (!plainPageClick(event)) return;
                            event.preventDefault();
                            choose(item);
                          }}
                          onKeyDownCapture={(event) => {
                            if (openModifiedPage(event.nativeEvent, href(item))) {
                              event.preventDefault();
                              event.stopPropagation();
                            } else if (
                              (event.key === 'Enter' || event.key === ' ') &&
                              !event.repeat &&
                              !event.altKey &&
                              !event.ctrlKey &&
                              !event.metaKey &&
                              !event.shiftKey
                            ) {
                              event.preventDefault();
                              event.stopPropagation();
                              choose(item);
                            }
                          }}
                        >
                          <span className="needs-you-header-row" title={item.title}>
                            {item.kind === 'answer' && item.questionCount ? (
                              <>
                                <span
                                  className="question-line-mark"
                                  data-state="waiting"
                                  aria-hidden="true"
                                >
                                  ◆
                                </span>{' '}
                                {item.questionCount.waiting} of {item.questionCount.total} questions
                                waiting
                                <span className="needs-you-header-agent"> · {item.title}</span>
                              </>
                            ) : (
                              item.title
                            )}
                            {item.agents.length > 0 && (
                              <span className="needs-you-header-agent">
                                {' · '}
                                <AgentList names={item.agents} />
                              </span>
                            )}
                          </span>
                        </a>
                      </DropdownMenuItem>
                    ))}
                  </DropdownMenuGroup>
                </Fragment>
              );
            })}
          </DropdownMenuContent>
        </DropdownMenu>
      </UiTheme>
    </span>
  );
}
