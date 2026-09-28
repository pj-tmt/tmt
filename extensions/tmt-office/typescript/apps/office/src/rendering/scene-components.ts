import { Container, Graphics } from 'pixi.js';
import { sceneLabel } from './scene-label.js';
import { componentActionBounds, componentAt } from './scene-component-geometry.js';
import type { SceneComponent } from './scene-component-geometry.js';

export function drawSceneComponents(parent: Container, components: readonly SceneComponent[]) {
  let scale = 16;
  const layer = new Container();
  parent.addChild(layer);
  const entries = components.map((component) => {
    const group = new Container();
    layer.addChild(group);
    const { bounds } = component;
    const base = new Graphics();
    group.addChild(base);
    // Artwork stays in the world's ordered placement pass. Only host-owned
    // affordances rise above it; a functional binding never changes paint order.
    const accents = new Graphics();
    group.addChild(accents);
    const plaque = new Graphics();
    group.addChild(plaque);
    const title = sceneLabel(
      group,
      '',
      bounds.x + 0.5,
      bounds.y - 1.7,
      0.8,
      '#eafff5',
      bounds.width - 1
    );
    function draw(active: boolean) {
      const color = component.available ? (active ? '#c2fff0' : '#70ddc6') : '#c7bda3';
      base
        .clear()
        .rect(bounds.x - 0.25, bounds.y + bounds.height - 1, bounds.width + 0.5, 0.75)
        .fill({ color, alpha: active ? 0.45 : 0.2 })
        .stroke({ color, width: active ? 0.25 : 0.15 });
      let action = componentActionBounds(component, scale, { expanded: active });
      const size = active
        ? Math.max(0.8, 12 / scale)
        : Math.min(Math.max(0.8, 12 / scale), action.width * 0.55);
      const padding = Math.max(0.5, 8 / scale);
      const symbol = component.available ? '↗' : '×';
      title.scale.set(size / 16);
      title.style.wordWrap = active;
      title.style.breakWords = true;
      title.style.wordWrapWidth = ((action.width - padding * 2) * 16) / size;
      title.text = active
        ? `${symbol} ${component.available ? component.label : 'Unavailable'}`
        : symbol;
      if (active) {
        // Measure the actual wrapped text once per transition/zoom. Picking uses
        // this same painted rectangle, never a guessed label height.
        action = componentActionBounds(component, scale, {
          expanded: true,
          contentHeight: title.height,
          contentWidth: title.width,
        });
        title.anchor.set(0);
        title.position.set(action.x + padding, action.y + Math.max(0.2, 5 / scale));
      } else {
        title.anchor.set(0.5);
        title.position.set(action.x + action.width / 2, action.y + action.height / 2);
      }
      entry.action = action;
      accents.clear();
      if (component.available) {
        const thickness = Math.max(0.16, 2 / scale);
        const arm = Math.min(1.2, bounds.width / 3, bounds.height / 3);
        for (const right of [false, true])
          for (const bottom of active ? [false, true] : [true]) {
            const x = bounds.x + (right ? bounds.width : 0);
            const y = bounds.y + (bottom ? bounds.height : 0);
            accents
              .rect(x - (right ? arm : 0), y - thickness / 2, arm, thickness)
              .rect(x - thickness / 2, y - (bottom ? arm : 0), thickness, arm)
              .fill(color);
          }
        if (active)
          accents
            .rect(bounds.x, bounds.y, bounds.width, bounds.height)
            .stroke({ color, width: thickness / 2 });
      }
      plaque
        .clear()
        .roundRect(
          action.x,
          action.y,
          action.width,
          action.height,
          Math.min(0.4, action.height / 4)
        )
        .fill({ color: component.available ? '#092b36' : '#333d41', alpha: active ? 0.98 : 0.88 })
        .stroke({ color, width: Math.max(0.12, 1 / scale) });
    }
    const entry = {
      id: component.id,
      available: component.available,
      body: bounds,
      action: componentActionBounds(component, scale),
      group,
      draw,
    };
    draw(false);
    return entry;
  });
  let highlighted: string | undefined;
  let ordered = entries;
  return {
    pick(x: number, y: number) {
      return componentAt(ordered, x, y);
    },
    zoom(next: number) {
      if (scale === next) return;
      scale = next;
      for (const entry of entries) entry.draw(entry.id === highlighted);
    },
    highlight(id?: string) {
      if (highlighted === id) return false;
      const previous = highlighted;
      highlighted = id;
      for (const entry of entries)
        if (entry.id === id || entry.id === previous) entry.draw(entry.id === id);
      const active = entries.find((entry) => entry.id === id);
      ordered = active ? [...entries.filter((entry) => entry !== active), active] : entries;
      // Expanded labels paint and pick above adjacent objects. Restore the
      // original order when idle; there is no independent z-index policy.
      for (const entry of ordered) layer.addChild(entry.group);
      return true;
    },
  };
}
