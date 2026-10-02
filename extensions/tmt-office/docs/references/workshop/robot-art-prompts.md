# Robot source prompts

Generated with the built-in imagegen tool. The selected RGBA source is
`robot-source-v2.png`; runtime pixels are in `workshop-robot-v2.tmtavatar.json`.
Mechanical compilation uses nearest-neighbor 32×48 sampling, nondithered palette
quantization, binary alpha and two-digit indices. Material roles distinguish
shell/body colors from fixed eyes, joints and the chest label; these roles are
bundled artwork metadata, not an extension of the public avatar format.

## Initial source

Use case: stylized-concept. Asset type: production pixel-art character sprite for a warm, richly furnished dollhouse office game. Create ONE friendly compact robot office worker, full body, front-facing with a subtly visible top plane, on a genuinely transparent background. Style: carefully hand-placed low-resolution pixel art designed on a 32 by 48 logical pixel canvas, enlarged with crisp nearest-neighbor square pixels, no vector curves, no soft antialiasing. Character occupies most of this portrait canvas with transparent padding all around. Large rounded-rectangular cream enamel head, recessed dark teal glass visor with two distinct mint illuminated eyes, small brass side ear fittings, a short antenna with a tiny mint cap. Small sturdy teal torso with a blank chest panel suitable for an application-rendered label, articulated brass shoulder and knee joints, small mechanical hands, short legs and substantial dark teal boots. Warm ivory shell highlights and restrained moss shadows give real dimensional detail. Charming and capable, matching an oak-and-moss pixel-art office. Consistent detailed silhouette, natural resting pose, readable eyes and joints at actual small sprite size. No text, no logo, no desk, no floor, no scenery, no cast shadow outside the silhouette, no separate props, no multiple variants, no UI, no watermark. Do not make a generic geometric outline or smooth 3D toy. This must be one isolated pixel-art game sprite, not a concept sheet.

## Cutout refinement

Use case: background-extraction. Edit only the background of the supplied robot sprite. Remove ALL black backdrop, ALL soft halo, glow, haze and drop shadows outside the character silhouette and replace them with genuinely transparent pixels (alpha=0). Keep the robot's design, exact silhouette, cream and teal materials, brass joints, eyes, pose, full body, blank chest panel and proportions unchanged. Preserve crisp pixel-stepped edges, do not soften them. Transparent holes between limbs and antenna must also be cut out. Do not add a checkerboard, solid color background, light bloom, floor, text, UI or any extra element. Export the isolated robot as an RGBA PNG with actual transparency, not a picture of a transparent background.

## Selected compact-body edit

Use case: precise-object-edit. Edit this robot into a compact chibi sprite for a 32x48 pixel office game. Preserve the same warm ivory enamel shell, teal visor, two mint eyes, brass joints, antenna and friendly design. Change proportions: head occupies about 60% of total character height, torso short and sturdy, legs extremely short with small chunky boots at the very bottom. The blank light cream chest label is centered at 75% of the full canvas height, with about 20% of canvas width available for application text. Resting arms compact beside torso. One full body robot only, centered in a portrait 2:3 canvas with 5% clear padding. Remove every shadow, halo or glow outside the character; background must be entirely transparent alpha=0 with clean cutout edges and transparent holes between limbs. Crisp pixel-art clusters, hard pixel steps, restrained warm shading. No text, no UI, no floor, no scenery, no multiple variants. This is an actual isolated reusable game sprite, not a poster. Keep antenna short; no hair or hats.
