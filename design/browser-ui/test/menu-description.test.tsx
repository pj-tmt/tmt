import { describe, expect, it, vi } from 'vite-plus/test';
import type { ReactElement } from 'react';
import { BrowserMenu, type BrowserMenuProps } from '../src/menu.js';
import { createMenuController } from '../src/menu-controller.js';

const hooks = vi.hoisted(() => ({ refs: [] as { current: unknown }[], index: 0 }));
vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  useId: () => 'fixture-menu',
  useRef: (value: unknown) => hooks.refs[hooks.index++] ?? { current: value },
  useSyncExternalStore: () => ({ open: true, present: true }),
  useLayoutEffect: () => undefined,
  useEffect: () => undefined,
}));

/** The menu rendered open, as its element tree: the trigger, then the dialog's `role="menu"` surface. */
function render(props: Partial<BrowserMenuProps<string>>, entries: unknown[] = []) {
  hooks.index = 0;
  hooks.refs = [{ current: null }, { current: null }, { current: entries }];
  const element = BrowserMenu({
    label: 'Fixture',
    items: [
      { key: 'one', label: 'One' },
      { key: 'two', label: 'Two' },
    ],
    onSelect() {},
    ...props,
  });
  const [trigger, dialog] = (element.props as { children: ReactElement[] }).children;
  const menu = (dialog.props as { children: ReactElement }).children;
  const [description, rows] = (menu.props as { children: [ReactElement | false, ReactElement[]] })
    .children;
  return { trigger, dialog, menu, description, rows };
}

describe('Menu description', () => {
  it('is the first block of the menu surface and its accessible description', () => {
    const { menu, description, rows } = render({ description: 'Open it on your phone' });
    const surface = menu.props as Record<string, unknown>;
    expect(surface.role).toBe('menu');
    expect(surface['aria-describedby']).toBe('fixture-menu-description');
    const block = description as ReactElement;
    expect(block.type).toBe('div');
    expect(block.props).toMatchObject({
      id: 'fixture-menu-description',
      className: 'tmt-ui-menu-description',
      children: 'Open it on your phone',
    });
    expect(rows).toHaveLength(2);
  });

  it('adds nothing to the menu without it', () => {
    const { menu, description } = render({});
    expect((menu.props as Record<string, unknown>)['aria-describedby']).toBeUndefined();
    expect(description).toBeFalsy();
  });

  it('is not an item and not a focus stop', () => {
    const { description } = render({ description: 'Informational' });
    const props = (description as ReactElement).props as Record<string, unknown>;
    for (const interactive of ['role', 'tabIndex', 'onClick', 'onKeyDown', 'onFocus'])
      expect(props[interactive], interactive).toBeUndefined();
  });

  it('does not move roving focus: arrows walk only the items', () => {
    const document = { activeElement: undefined as unknown };
    const entry = () => ({ ownerDocument: document, focus: vi.fn() });
    const [first, second] = [entry(), entry()];
    document.activeElement = first;
    const controller = createMenuController({
      items: () => [
        { key: 'one', label: 'One' },
        { key: 'two', label: 'Two' },
      ],
      disabled: () => false,
      hasDescription: () => true,
      trigger: () => null,
      surface: () => null,
      entries: () => [first, second] as unknown as HTMLButtonElement[],
      selection: () => () => {},
      onOpenChange: () => {},
    });
    controller.open();
    const press = (key: string) => {
      const event = {
        key,
        defaultPrevented: false,
        isTrusted: true,
        altKey: false,
        ctrlKey: false,
        metaKey: false,
        nativeEvent: { isComposing: false },
        preventDefault: vi.fn(),
        stopPropagation: vi.fn(),
      };
      controller.menuKey(event);
      return event;
    };
    expect(press('ArrowDown').preventDefault).toHaveBeenCalled();
    expect(second.focus).toHaveBeenCalledTimes(1);
    document.activeElement = second;
    press('ArrowDown');
    expect(first.focus).toHaveBeenCalledTimes(1);
    press('Home');
    expect(first.focus).toHaveBeenCalledTimes(2);
  });

  it('lets a menu with only a description open, and a menu with nothing to show stay shut', () => {
    expect(render({ items: [], description: 'Only text' }).trigger.props).toMatchObject({
      disabled: false,
    });
    expect(render({ items: [] }).trigger.props).toMatchObject({ disabled: true });
    expect(
      render({ items: [{ key: 'one', label: 'One', disabled: true }] }).trigger.props,
    ).toMatchObject({ disabled: true });
  });
});
