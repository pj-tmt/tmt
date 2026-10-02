"""Encode reviewed source art into existing v2 prop/avatar packs, not a runtime loader.

Requires Pillow 12.3.0. Source PNGs are read-only. Defaults to checking exact output;
--write explicitly regenerates the manifest's contract file. Native/browser prop
validators remain the admission authority.
"""

import argparse
import hashlib
import json
from pathlib import Path

from PIL import Image


def encode(manifest_path: Path) -> tuple[Path, bytes]:
    spec = json.loads(manifest_path.read_text())

    def reviewed_source(description):
        source_path = manifest_path.parent / description["source"]
        if hashlib.sha256(source_path.read_bytes()).hexdigest() != description["sha256"]:
            raise ValueError("Source changed; review crop bounds before regenerating.")
        source = Image.open(source_path).convert("RGBA")
        if list(source.size) != description["size"]:
            raise ValueError("Source dimensions do not match the reviewed sheet.")
        return source

    source = reviewed_source(spec)
    frames_by_prop = []
    avatar = "avatars" in spec
    entries = spec["avatars" if avatar else "props"]
    for prop in entries:
        sheet = reviewed_source(prop["sheet"]) if "sheet" in prop else source
        side = prop.get("side", 32)
        if not avatar and (not 1 <= side <= 128 or side % 8):
            raise ValueError("Frame side must fit the v2 bound at eight pixels per tile.")
        if len(prop["crops"]) not in ((1,) if avatar else (1, 4)):
            raise ValueError("Provide one static view or four authored orientations.")
        frame_width, frame_height = (32, 48) if avatar else (side, side)
        sprites = []
        for crop in prop["crops"]:
            x, y, right, bottom = crop
            if not (0 <= x < right <= sheet.width and 0 <= y < bottom <= sheet.height):
                raise ValueError("Crop is outside the reviewed source.")
            sprites.append(sheet.crop(crop))
        scale = min((frame_width - 2) / max(s.width for s in sprites),
                    (frame_height - 2) / max(s.height for s in sprites))
        frames = []
        for sprite in sprites:
            # Preserve aspect ratio and a transparent guard pixel. The square
            # footprint keeps static billboards undistorted across rotations.
            if prop.get("sharedScale", False):
                sprite = sprite.resize((max(1, round(sprite.width * scale)),
                                        max(1, round(sprite.height * scale))),
                                       Image.Resampling.LANCZOS)
            else:
                sprite.thumbnail((frame_width - 2, frame_height - 2), Image.Resampling.LANCZOS)
            frame = Image.new("RGBA", (frame_width, frame_height))
            frame.paste(sprite, ((frame_width - sprite.width) // 2, frame_height - sprite.height - 1))
            frames.append(frame)
        frames_by_prop.append(frames if avatar or len(frames) == 4 else frames * 4)

    # One deterministic palette for the pack, with index zero reserved for alpha.
    # V2 admits binary transparency only; this explicit derivative is reviewed
    # beside the unchanged source rather than claiming lossless PNG admission.
    all_frames = [frame for frames in frames_by_prop for frame in frames]
    master = Image.new("RGB", (max(frame.width for frame in all_frames), sum(
        frame.height for frame in all_frames
    )), (110, 76, 44))
    offset = 0
    for frame in all_frames:
        mask = frame.getchannel("A").point(lambda alpha: 255 if alpha >= 128 else 0)
        master.paste(frame.convert("RGB"), (0, offset), mask)
        offset += frame.height
    colors = master.quantize(colors=255, method=Image.Quantize.MEDIANCUT,
                             dither=Image.Dither.NONE)
    palette = colors.getpalette()[:255 * 3]
    document = {
        "formatVersion": 2,
        "label": spec["label"],
        "credit": spec.get("credit", "TMT · generated modular-v1 source artwork"),
        "license": "MIT",
        "palette": ["#00000000"] + [
            "#" + bytes(palette[i:i + 3]).hex() + "ff" for i in range(0, len(palette), 3)
        ],
        "avatars" if avatar else "props": [],
    }
    offset = 0
    for prop, frames in zip(entries, frames_by_prop, strict=True):
        rows = []
        for frame in frames:
            alpha = frame.getchannel("A")
            rows.append([
                "".join(f"{colors.getpixel((x, y + offset)) + 1:02x}" if alpha.getpixel((x, y)) >= 128
                        else "00" for x in range(frame.width))
                for y in range(frame.height)
            ])
            offset += frame.height
        entry = {"key": prop["key"], "label": prop["label"]}
        if avatar:
            entry["pixels"] = rows[0]
        else:
            entry.update(footprint={"width": prop["side"] // 8, "height": prop["side"] // 8}, frames=rows)
        document["avatars" if avatar else "props"].append(entry)
    payload = (json.dumps(document, ensure_ascii=False, indent=2) + "\n").encode()
    byte_limit, cell_limit = (32 * 1024, 6_144) if avatar else (512 * 1024, 131_072)
    if len(payload) > byte_limit or sum(f.width * f.height for f in all_frames) > cell_limit:
        raise ValueError("Encoded pack exceeds v2 admission budgets.")
    return (manifest_path.parent / spec["output"]).resolve(), payload


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    output, payload = encode(args.manifest.resolve())
    if args.write:
        output.write_bytes(payload)
    elif not output.is_file() or output.read_bytes() != payload:
        raise SystemExit(f"Generated pack differs: {output}")
    domain = b"TMT-OFFICE-AVATAR-PACK-V2\0" if "avatars" in json.loads(payload) else b"TMT-OFFICE-PROP-PACK-V2\0"
    digest = hashlib.sha256(domain + len(payload).to_bytes(8, "big") + payload)
    print(f"{output.name}: {len(payload)} bytes, sha256:{digest.hexdigest()}")
