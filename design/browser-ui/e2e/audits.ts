import type { Page } from '@playwright/test';

/*
 * Computed-style audits of the real CSS in a real browser. Each function runs inside the page, so
 * it is self-contained; `scope` is a selector for the part of the page to audit.
 */

/** A rounded box that draws one, two or three sides of its border, unequal sides, or an inset
 * edge: a one-sided border curls at the corners, so the box needs a full border, a fill, or a
 * separate straight indicator. */
export function oneSidedBorders(page: Page, scope: string) {
  return page.evaluate((root) => {
    const sides = ['Top', 'Right', 'Bottom', 'Left'] as const;
    const corners = ['TopLeft', 'TopRight', 'BottomRight', 'BottomLeft'] as const;
    const out = new Set<string>();
    for (const element of document.querySelectorAll(`${root} *`)) {
      for (const pseudo of [null, '::before', '::after'] as const) {
        const style = getComputedStyle(element, pseudo);
        if (pseudo && style.content === 'none') continue;
        if (!corners.some((corner) => parseFloat(style[`border${corner}Radius`]) > 0)) continue;
        const drawn = sides.map((side) =>
          style[`border${side}Style`] !== 'none' && parseFloat(style[`border${side}Width`]) > 0
            ? 1
            : 0,
        );
        const count = drawn.reduce<number>((sum, side) => sum + side, 0);
        const unequal =
          count === 4 &&
          new Set(sides.map((side) => style[`border${side}Width`] + style[`border${side}Color`]))
            .size > 1;
        const inset = /inset/.test(style.boxShadow);
        if ((count !== 0 && count !== 4) || unequal || inset)
          out.add(
            `${element.tagName.toLowerCase()}.${String(element.className).split(' ').slice(0, 2).join('.')}${pseudo ?? ''} sides=${drawn.join('')}${inset ? ' inset' : ''}`,
          );
      }
    }
    return [...out];
  }, scope);
}

/** A filled or bordered box whose text starts within 3px of its own edge. */
export function textAtSurfaceEdge(page: Page, scope: string) {
  return page.evaluate((root) => {
    const out = new Set<string>();
    const fills = (style: CSSStyleDeclaration) => {
      const match = /rgba?\(([^)]+)\)/.exec(style.backgroundColor);
      return match ? Number(match[1]!.split(',')[3] ?? 1) > 0.05 : false;
    };
    const drawn = (style: CSSStyleDeclaration, side: 'Left' | 'Right') =>
      style[`border${side}Style`] !== 'none' && parseFloat(style[`border${side}Width`]) > 0;
    // Visually hidden text (a chart's data table) is clipped away, not drawn on a surface.
    const hidden = (element: HTMLElement | null): boolean =>
      !!element &&
      (getComputedStyle(element).clipPath !== 'none' ||
        getComputedStyle(element).clip !== 'auto' ||
        getComputedStyle(element).visibility === 'hidden' ||
        hidden(element.parentElement));
    const walker = document.createTreeWalker(document.querySelector(root)!, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (!node.textContent!.trim()) continue;
      const range = document.createRange();
      range.selectNodeContents(node);
      const text = range.getBoundingClientRect();
      const parent = node.parentElement!;
      if (!text.width || hidden(parent)) continue;
      let visibleLeft = text.left;
      let visibleRight = text.right;
      for (let element: HTMLElement | null = parent; element; element = element.parentElement) {
        const style = getComputedStyle(element);
        const up = element.parentElement ? getComputedStyle(element.parentElement) : undefined;
        const filled = fills(style) && style.backgroundColor !== up?.backgroundColor;
        const bordered = drawn(style, 'Left') && drawn(style, 'Right');
        const box = element.getBoundingClientRect();
        if (!filled && !bordered) {
          // A transparent scroll/clip container can hide glyphs before they reach a more
          // distant filled surface. Range bounds include those unpainted glyphs.
          if (style.overflowX !== 'visible' && element.clientWidth > 0) {
            const clipLeft = box.left + element.clientLeft;
            visibleLeft = Math.max(visibleLeft, clipLeft);
            visibleRight = Math.min(visibleRight, clipLeft + element.clientWidth);
            if (visibleRight <= visibleLeft) break;
          }
          continue;
        }
        const left = visibleLeft - box.left - (parseFloat(style.borderLeftWidth) || 0);
        const right = box.right - visibleRight - (parseFloat(style.borderRightWidth) || 0);
        // Text outside its surface (scrolled away) is not on it.
        if (box.width > 4 && left > -1 && right > -1 && (left < 3 || right < 3))
          out.add(
            `${element.tagName.toLowerCase()}.${String(element.className).split(' ').slice(0, 3).join('.')} left=${left.toFixed(0)} right=${right.toFixed(0)} «${node.textContent!.trim().slice(0, 30)}»`,
          );
        break;
      }
    }
    // A tick glyph inside a toggle indicator is a mark in a box, not text on a surface.
    return [...out].filter((entry) => !entry.startsWith('span.tmt-ui-toggle-indicator'));
  }, scope);
}

/** Every field, action, icon action and menu trigger takes the one radius token; the token itself
 * is returned so a test can pin its value. */
export function controlRadii(page: Page, scope: string) {
  return page.evaluate((root) => {
    const probe = document.createElement('div');
    probe.style.borderRadius = 'var(--tmt-ui-radius)';
    document.body.append(probe);
    const token = getComputedStyle(probe).borderTopLeftRadius;
    probe.remove();
    const selector = [
      '.tmt-ui-field-control',
      '.tmt-ui-action',
      '.tmt-ui-icon-action-control',
      '.tmt-ui-menu-trigger:not([data-variant="crumb"])',
      '.tmt-ui-chips-add',
    ]
      .map((part) => `${root} ${part}`)
      .join(',');
    const off = new Map<string, string>();
    for (const element of document.querySelectorAll(selector)) {
      // A chip's remove control is a straight segment inside the chip's own rounded edge, and the
      // UI layer sets its own radii.
      if (element.closest('.tmt-ui-chip, .tmt-ui-tabs, [data-tmt-ui]')) continue;
      const style = getComputedStyle(element);
      if (style.display === 'none') continue;
      if (
        style.borderTopLeftRadius !== token ||
        style.borderTopRightRadius !== token ||
        style.borderBottomRightRadius !== token ||
        style.borderBottomLeftRadius !== token
      )
        off.set(
          `${element.tagName.toLowerCase()}.${String(element.className).split(' ').slice(0, 2).join('.')}`,
          style.borderTopLeftRadius,
        );
    }
    return { token, off: [...off] };
  }, scope);
}

/** Focus frames: (a) a ring that a rounded overflow ancestor would clip is drawn by that ancestor;
 * (b) a control inside a focus-carrying ancestor draws no outline, border colour change, fill or
 * shadow of its own, so one focus frame shows. Returns how many controls were checked. */
export function focusFrames(page: Page, scope: string) {
  return page.evaluate((root) => {
    const clipped = new Set<string>();
    const doubled = new Set<string>();
    const colour = (element: Element, token: string) => {
      const probe = document.createElement('span');
      probe.style.color = `var(${token})`;
      element.append(probe);
      const value = getComputedStyle(probe).color;
      probe.remove();
      return value;
    };
    const corners = {
      TopLeft: [0, 0, 1, 1],
      TopRight: [1, 0, -1, 1],
      BottomRight: [1, 1, -1, -1],
      BottomLeft: [0, 1, 1, -1],
    } as const;
    type Corner = keyof typeof corners;
    const radiusOf = (style: CSSStyleDeclaration, corner: Corner) =>
      parseFloat(style[`border${corner}Radius`]) || 0;
    const inside = (x: number, y: number, box: DOMRect, style: CSSStyleDeclaration) => {
      for (const corner of Object.keys(corners) as Corner[]) {
        const [fx, fy, dx, dy] = corners[corner];
        const radius = radiusOf(style, corner);
        if (!radius) continue;
        const along = (x - (box.left + fx * box.width)) * dx;
        const across = (y - (box.top + fy * box.height)) * dy;
        if (along >= 0 && along < radius && across >= 0 && across < radius)
          return Math.hypot(along - radius, across - radius) <= radius;
      }
      return true;
    };
    /** What a focused control draws beyond its idle self: an outline, a changed border colour, a
     * new fill or a shadow. */
    const drawn = (element: Element) => {
      const style = getComputedStyle(element);
      return {
        outline:
          style.outlineStyle === 'none' || parseFloat(style.outlineWidth) === 0
            ? ''
            : style.outlineColor,
        border: [
          style.borderTopColor,
          style.borderRightColor,
          style.borderBottomColor,
          style.borderLeftColor,
        ].join('|'),
        fill: style.backgroundColor,
        shadow: style.boxShadow === 'none' ? '' : style.boxShadow,
      };
    };
    const adds = (before: ReturnType<typeof drawn>, after: ReturnType<typeof drawn>) =>
      Boolean(after.outline) ||
      Boolean(after.shadow) ||
      after.border !== before.border ||
      (after.fill !== before.fill && after.fill !== 'rgba(0, 0, 0, 0)');
    let checked = 0;
    for (const element of document.querySelectorAll<HTMLElement>(
      `${root} :is(button, a[href], input, select, textarea)`,
    )) {
      const box = element.getBoundingClientRect();
      if (!box.width || element.closest('[hidden], dialog:not([open])')) continue;
      element.blur();
      const before = drawn(element);
      const ancestors: HTMLElement[] = [];
      for (let up = element.parentElement; up; up = up.parentElement) ancestors.push(up);
      element.focus();
      if (!element.matches(':focus-visible')) continue;
      checked++;
      const own = getComputedStyle(element);
      const ring = (Object.keys(corners) as Corner[]).map((corner) => {
        const [fx, fy, dx, dy] = corners[corner];
        const inset = radiusOf(own, corner) * (1 - Math.SQRT1_2);
        return [
          box.left + fx * box.width + dx * inset,
          box.top + fy * box.height + dy * inset,
        ] as const;
      });
      const focus = colour(element, '--tmt-ui-color-focus');
      const focusDestructive = colour(element, '--tmt-ui-color-focus-destructive');
      let carried = false;
      ancestors.forEach((ancestor) => {
        const style = getComputedStyle(ancestor);
        if (style.overflowX === 'visible' && style.overflowY === 'visible') return;
        if (!parseFloat(style.borderTopLeftRadius)) return;
        const outer = ancestor.getBoundingClientRect();
        // A control scrolled out of its container is not in a corner at all.
        const within = ring.every(
          ([x, y]) => x >= outer.left && x <= outer.right && y >= outer.top && y <= outer.bottom,
        );
        if (!within || ring.every(([x, y]) => inside(x, y, outer, style))) return;
        const draws = style.borderTopColor === focus || style.borderTopColor === focusDestructive;
        if (draws) carried = true;
        else clipped.add(`${name(element)} in ${name(ancestor)}`);
      });
      // A focus-carrying ancestor's frame is the only one: the control changes nothing itself.
      if (carried && adds(before, drawn(element))) doubled.add(name(element));
      element.blur();
    }
    function name(element: Element) {
      return `${element.tagName.toLowerCase()}.${String(element.className).split(' ').slice(0, 2).join('.')}`;
    }
    return { checked, clipped: [...clipped], doubled: [...doubled] };
  }, scope);
}

/** Text fields and select triggers keep the soft v2 rule edge (gui-style.md, "Fields"): the border
 * rule skips them by name, and the field rule holds them to the conditions that make the soft edge
 * acceptable. */
export const fieldsKeepSoftEdge = [
  '.tmt-ui-field-control',
  '.tmt-ui-menu-trigger[data-variant="select"]',
] as const;

/**
 * Control boundaries (WCAG 1.4.11), measured in a real browser against the opaque surface behind
 * each control:
 * - `borders`: a control drawn by its border (an unchecked checkbox, a radio, a toggle's
 *   indicator) keeps that border at 3:1 or more. Fields are skipped by name (`fieldsKeepSoftEdge`).
 * - `fields`: a text field or select trigger fills differently from its surface and carries a
 *   visible label (a field), a visible value (a select) or a visible placeholder (a search field),
 *   so the soft edge is not the only cue.
 */
export function controlBoundaries(page: Page, scope: string) {
  return page.evaluate(
    ({ root, soft }) => {
      const canvas = document.createElement('canvas');
      canvas.width = canvas.height = 1;
      const context = canvas.getContext('2d', { willReadFrequently: true })!;
      /** Any computed CSS colour as sRGB plus alpha, through the canvas. */
      const rgba = (value: string): [number, number, number, number] => {
        context.clearRect(0, 0, 1, 1);
        context.fillStyle = '#000';
        context.fillStyle = value;
        context.fillRect(0, 0, 1, 1);
        const [r, g, b, a] = context.getImageData(0, 0, 1, 1).data;
        return [r!, g!, b!, a! / 255];
      };
      const over = (top: number[], bottom: number[]): [number, number, number, number] => {
        const alpha = top[3]!;
        return [
          top[0]! * alpha + bottom[0]! * (1 - alpha),
          top[1]! * alpha + bottom[1]! * (1 - alpha),
          top[2]! * alpha + bottom[2]! * (1 - alpha),
          1,
        ];
      };
      /** The opaque colour behind an element: its ancestors' fills composited from the page up. */
      const backdrop = (element: Element) => {
        const fills: [number, number, number, number][] = [];
        for (let up = element.parentElement; up; up = up.parentElement) {
          const fill = rgba(getComputedStyle(up).backgroundColor);
          if (fill[3] > 0) fills.push(fill);
          if (fill[3] === 1) break;
        }
        return fills.reduceRight<[number, number, number, number]>(
          (below, fill) => over(fill, below),
          [255, 255, 255, 1],
        );
      };
      const luminance = (colour: number[]) => {
        const [r, g, b] = colour.slice(0, 3).map((channel) => {
          const unit = channel! / 255;
          return unit <= 0.03928 ? unit / 12.92 : ((unit + 0.055) / 1.055) ** 2.4;
        });
        return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
      };
      const ratio = (a: number[], b: number[]) => {
        const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x);
        return (light! + 0.05) / (dark! + 0.05);
      };
      const visible = (element: Element) =>
        getComputedStyle(element).display !== 'none' && element.getBoundingClientRect().width > 0;
      const name = (element: Element) =>
        `${element.tagName.toLowerCase()}.${String(element.className).split(' ')[0]}`;

      const borders = { measured: 0, misses: new Set<string>() };
      const checks = [
        '.tmt-ui-checkbox:not(:checked):not(:indeterminate):not(:disabled)',
        '.tmt-ui-radio:not(:checked):not(:disabled)',
        '.tmt-ui-toggle:not([aria-pressed="true"]):not(:disabled) .tmt-ui-toggle-indicator',
      ]
        .map((part) => `${root} ${part}`)
        .join(',');
      for (const element of document.querySelectorAll(checks)) {
        if (soft.some((part) => element.matches(part)) || !visible(element)) continue;
        const edge = rgba(getComputedStyle(element).borderTopColor);
        const behind = backdrop(element);
        borders.measured++;
        const found = ratio(over(edge, behind), behind);
        if (found < 3) borders.misses.add(`${name(element)} ${found.toFixed(2)}:1`);
      }

      const fields = { measured: 0, sameFill: new Set<string>(), unlabelled: new Set<string>() };
      for (const element of document.querySelectorAll<HTMLElement>(
        soft.map((part) => `${root} ${part}`).join(','),
      )) {
        if (!visible(element)) continue;
        fields.measured++;
        const behind = backdrop(element);
        const fill = over(rgba(getComputedStyle(element).backgroundColor), behind);
        // Any visible step: identical or within rounding is no step at all.
        if (ratio(fill, behind) < 1.04) fields.sameFill.add(name(element));
        const labels = [...((element as HTMLInputElement).labels ?? [])];
        const labelledBy = (element.getAttribute('aria-labelledby') ?? '')
          .split(/\s+/)
          .map((id) => document.getElementById(id))
          .filter((node): node is HTMLElement => node !== null);
        const named = [...labels, ...labelledBy].some(
          (label) => visible(label) && label.textContent!.trim() !== '',
        );
        // A select trigger shows its value (and the chevron the CSS draws) instead of a label; a
        // search field shows its placeholder beside the magnifier the CSS draws.
        const shownValue =
          (element.matches('button') && element.textContent!.trim() !== '') ||
          (element.matches('input[type="search"]') &&
            (element as HTMLInputElement).placeholder.trim() !== '');
        if (!named && !shownValue) fields.unlabelled.add(name(element));
      }
      return {
        borders: { measured: borders.measured, misses: [...borders.misses] },
        fields: {
          measured: fields.measured,
          sameFill: [...fields.sameFill],
          unlabelled: [...fields.unlabelled],
        },
      };
    },
    { root: scope, soft: [...fieldsKeepSoftEdge] },
  );
}
