# Commons noticeboard artwork

Runtime artifact: [commons-props-v2.tmtprop.json](../../../contracts/commons-props-v2.tmtprop.json).
The pack contains one freestanding noticeboard with four independently authored
upright views. It grants no discussion capability by itself.

Generated with the built-in OpenAI imagegen tool, then mechanically resampled
into the existing workshop palette and transparent indexed prop format. Front
and back frames are 96 × 64; side frames are 64 × 96. Visible object height is
held consistent across directions. The runtime JSON is self-contained; generated
source and review previews under `target/workshop-art/` are local build artifacts.

## Generation prompt

Use case: stylized-concept. Asset type: transparent game sprite sheet for a warm
pixel-art agent office. Generate one freestanding community noticeboard in four
upright elevated orthographic views, arranged in a clean 2-by-2 sheet: South/front
top-left, West/left side top-right, North/back bottom-left, East/right side
bottom-right. Same physical noticeboard in all views: rich honey-oak frame and
short sturdy feet, forest-green felt board with a few small cream paper notes
attached with muted terracotta pins; back has wooden planks and support brace.
Hand-placed-looking crisp pixel clusters, restrained 24-color appearance, warm
upper-left highlights, dark moss/brown outlines, cozy detailed retro workshop art
rather than flat geometric illustration. Target final per-frame footprint 12x8
tiles, front/back raster96x64 pixels; side views raster64x96 pixels. Entire frame
including feet must fit with at least15% clear transparent padding in each
quadrant. Actual transparent alpha background, no checkerboard baked in. No floor,
wall, room, other props, people, labels, readable words, logos or watermark. Camera
elevation consistent across views; do not simply rotate one raster. Clearly
separate four complete objects without overlap.
