import {
  useLayoutEffect,
  useRef,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from 'react';
import {
  browserUiClasses as c,
  countText,
  type BrowserTab,
  type BrowserTabSelectReason,
} from './static.js';
export type { BrowserTab, BrowserTabSelectReason } from './static.js';
export interface BrowserTabsProps {
  label: string;
  tabs: readonly BrowserTab[];
  selected: string;
  onSelect(
    id: string,
    reason: BrowserTabSelectReason,
    event: KeyboardEvent<HTMLButtonElement> | MouseEvent<HTMLButtonElement>,
  ): void;
  status?: ReactNode;
  /** `boxed` (default) frames the strip and its panels; `flush` is a strip with an underline over
   * content that sits on the page, for a list a few views of one set share; `flat` has no strip
   * frame at all: each tab is its own pill and the selected one is filled and edged. */
  variant?: 'boxed' | 'flush' | 'flat';
  /** `flat` only: the strip wraps into rows instead of scrolling sideways, for a list whose tabs
   * should all show on a phone. The arrow-key order stays the DOM order. Scroll stays the default. */
  wrap?: boolean;
  /** Flat tabs only: website-sized labels/counts with a neutral selected surface. */
  presentation?: 'compact';
  panelTabIndex?: 0 | -1;
  children(tab: BrowserTab): ReactNode;
}

/** The tab's accessible name: its label, its count and its state word; colour alone says nothing. */
function tabName(tab: BrowserTab): string {
  const label = tab.count === undefined ? tab.label : `${tab.label}, ${countText(tab.count)}`;
  return tab.state ? `${tab.state.mark} ${label} · ${tab.state.word}` : label;
}

/** Selection and panel lifetimes belong to the host. Every panel stays mounted. */
export function BrowserTabs({
  label,
  tabs,
  selected,
  onSelect,
  status,
  variant = 'boxed',
  wrap = false,
  presentation,
  panelTabIndex,
  children,
}: BrowserTabsProps) {
  const strip = useRef<HTMLDivElement>(null);
  const tabIds = JSON.stringify(tabs.map(({ id, tabId }) => [id, tabId]));
  useLayoutEffect(() => {
    const list = strip.current;
    const tab = list?.querySelector<HTMLElement>('[aria-selected="true"]');
    if (!list || !tab) return;
    // Move this strip only: scrollIntoView would also move the host's page.
    const keepVisible = () => {
      const bounds = list.getBoundingClientRect();
      const item = tab.getBoundingClientRect();
      if (item.left < bounds.left) list.scrollLeft += item.left - bounds.left;
      else if (item.right > bounds.right) list.scrollLeft += item.right - bounds.right;
    };
    keepVisible();
    const observer = new ResizeObserver(keepVisible);
    observer.observe(list);
    observer.observe(tab);
    return () => observer.disconnect();
  }, [selected, tabIds]);
  function key(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    let next: number;
    switch (event.key) {
      case 'ArrowLeft':
        next = (index + tabs.length - 1) % tabs.length;
        break;
      case 'ArrowRight':
        next = (index + 1) % tabs.length;
        break;
      case 'Home':
        next = 0;
        break;
      case 'End':
        next = tabs.length - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    const target = strip.current?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next];
    target?.focus({ preventScroll: true });
    onSelect(tabs[next].id, 'keyboard', event);
  }
  return (
    <div
      className={c.tabs}
      data-variant={variant}
      data-presentation={variant === 'flat' ? presentation : undefined}
      data-wrap={wrap && variant === 'flat' ? '' : undefined}
    >
      <div className={c.tabsHeader}>
        <div className={c.tabList} role="tablist" aria-label={label} ref={strip}>
          {tabs.map((tab, index) => (
            <button
              key={tab.id}
              id={tab.tabId}
              type="button"
              className={c.tab}
              role="tab"
              aria-controls={tab.panelId}
              aria-selected={tab.id === selected}
              aria-label={tabName(tab)}
              tabIndex={tab.id === selected ? 0 : -1}
              onKeyDown={(event) => key(event, index)}
              onClick={(event) => onSelect(tab.id, 'activation', event)}
            >
              {tab.state && (
                <span className={c.tabMark} data-tone={tab.state.tone} aria-hidden="true">
                  {tab.state.mark}
                </span>
              )}
              <span className={c.tabLabel} aria-hidden="true">
                {tab.label}
              </span>
              {tab.count !== undefined && (
                <span className={c.tabCount} aria-hidden="true">
                  {countText(tab.count)}
                </span>
              )}
            </button>
          ))}
        </div>
        {status !== undefined && <div className={c.tabsStatus}>{status}</div>}
      </div>
      {tabs.map((tab) => (
        <div
          key={tab.id}
          id={tab.panelId}
          className={c.tabPanel}
          role="tabpanel"
          aria-labelledby={tab.tabId}
          tabIndex={panelTabIndex}
          hidden={tab.id !== selected}
        >
          {children(tab)}
        </div>
      ))}
    </div>
  );
}
