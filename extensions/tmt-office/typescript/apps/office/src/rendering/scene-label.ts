import { Container, Text } from 'pixi.js';

/** Rasterize readable glyphs once, then place them in world tile units. */
export function sceneLabel(
  parent: Container,
  text: string,
  x: number,
  y: number,
  size: number,
  color = '#f6edcf',
  wrapWidth = 28
) {
  const fontSize = 16;
  const item = new Text({
    text,
    style: {
      fontFamily: 'system-ui',
      fontSize,
      fill: color,
      wordWrap: true,
      wordWrapWidth: (wrapWidth * fontSize) / size,
    },
  });
  item.scale.set(size / fontSize);
  item.position.set(x, y);
  parent.addChild(item);
  return item;
}
