# Whiteboard artwork

Generated with the built-in imagegen tool. The [source sheet](whiteboard-source-v1.png)
contains four views of one object, not world geometry or executable behavior.

## Final prompt

Use case: stylized-concept. Asset type: production pixel-art furniture sprite sheet for TMT Office. Generate a SQUARE transparent image showing ONE freestanding oak-and-teal whiteboard in FOUR upright directional views arranged in a strict equal 2 by 2 grid: top-left SOUTH/front, top-right WEST/left side, bottom-left NORTH/back, bottom-right EAST/right side. Warm detailed 16-bit cozy office game art, deliberate crisp pixel clusters, honey-oak wood grain, dark teal metal brackets, cream ceramic writing surface, little brass bolts, wooden marker tray with three small muted moss/terracotta/teal markers and an eraser, solid twin wooden posts and short stable feet. The front contains a few faint muted hand-drawn boxes/arrows and a tiny cream sticky note, not legible words. The back shows actual wooden rear panels and brace; side views show thin board depth and posts, not duplicate fronts. Elevated 3/4 top-down camera like a cozy pixel office, NOT an isometric diamond. Keep identical actual object scale, height and camera elevation in ALL four views; consistent material, upper-left baked highlights and dark teal-brown outlines. Board must feel richly textured and substantial, not a flat vector diagram. CRITICAL framing: each sprite is centered in its own SQUARE quadrant and fits inside the central 80 percent width AND central 60 percent height of that quadrant, including feet. The same roughly 55 percent cell-height in each view, generous transparent margins above/below and around, no cross-quadrant elements. Side silhouettes naturally narrower, NOT taller or magnified. Designed for mechanical nearest-neighbor reduction of each square quadrant to 96x96 source pixels, then retaining the center 96x64 for front/back and center 64x96 for sides; NOTHING outside those rectangles. Genuine transparent alpha, no fake checkerboard, no background, no floor, no external cast shadow. No words, labels, grid lines, letters, numbers, HUD, logos, people, robots, bloom, blur or glossy 3D rendering. Only these four views of the same authored furniture object.

## Indexed compilation

The generated image is 1254 × 1254. Its actual view regions are separated at
`x = 750` and `y = 627`; they are not equal quadrants. Preserve each complete
silhouette rather than cropping at the requested but unrealized midpoint.
Using alpha ≥ 128, the source bounds in South/West/North/East order are
`(145,115,529,441)`, `(899,106,191,461)`, `(145,692,529,439)` and
`(875,683,185,455)` as `(x,y,width,height)`.

All views use one scale, `60 / 441`, with nearest-neighbor reduction and centered
placement into 96 × 64 / 64 × 96 / 96 × 64 / 64 × 96 rasters. A shared,
non-dithered 255-color quantization produces the admitted 247-entry palette;
alpha below 128 maps to transparent index zero, otherwise opaque. The runtime
[pack](../../../contracts/whiteboard-props-v2.tmtprop.json) contains
24,576 cells and 58,319 exact source bytes, not the source PNG. Its 12 × 8 tile
footprint retains eight source pixels per tile in all directions.

The faint marks belong to the artwork; they are not a projection of a saved
whiteboard document. The World adds interaction affordances and the registered
host binding opens the real editor.
