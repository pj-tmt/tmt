"""Offline encoder reproducibility and source-review fences; no runtime admission clone."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "docs/office/references/rooms-and-walls/modular-v1/workstation-import.json"
MODULE_SPEC = importlib.util.spec_from_file_location("encode_props", Path(__file__).with_name("encode-props.py"))
encoder = importlib.util.module_from_spec(MODULE_SPEC)
MODULE_SPEC.loader.exec_module(encoder)


class PropEncodingTests(unittest.TestCase):
    def test_directional_imports_reproduce_exact_packs(self):
        directory = ROOT / "docs/office/references/furniture-rotation"
        for manifest in directory.glob("*-import.json"):
            with self.subTest(manifest=manifest.name):
                output, encoded = encoder.encode(manifest)
                self.assertEqual(output.read_bytes(), encoded)

    def test_checked_in_pack_is_exact_and_source_is_not_modified(self):
        for name in ("workstation-import.json", "mounted-import.json", "lounge-import.json", "robot-import.json", "facilities-import.json", "reception-import.json"):
            with self.subTest(manifest=name):
                manifest = MANIFEST.with_name(name)
                spec = json.loads(manifest.read_text())
                source = manifest.parent / spec["source"]
                original = source.read_bytes()
                output, encoded = encoder.encode(manifest)
                self.assertEqual(output.read_bytes(), encoded)
                self.assertEqual(source.read_bytes(), original)

    def test_robot_pack_uses_static_avatar_dimensions(self):
        _, encoded = encoder.encode(MANIFEST.with_name("robot-import.json"))
        pack = json.loads(encoded)
        self.assertNotIn("props", pack)
        self.assertEqual(len(pack["avatars"]), 4)
        for avatar in pack["avatars"]:
            self.assertEqual(len(avatar["pixels"]), 48)
            self.assertTrue(all(len(row) == 64 for row in avatar["pixels"]))

    def rejected(self, edit, message):
        spec = json.loads(MANIFEST.read_text())
        spec["source"] = str(MANIFEST.parent / spec["source"])
        edit(spec)
        with tempfile.TemporaryDirectory() as directory:
            temporary = Path(directory) / "manifest.json"
            temporary.write_text(json.dumps(spec))
            with self.assertRaisesRegex(ValueError, message):
                encoder.encode(temporary)

    def test_changed_source_requires_new_review(self):
        self.rejected(lambda spec: spec.update(sha256="0" * 64), "Source changed")

    def test_dimensions_cannot_silently_rescale_crops(self):
        self.rejected(lambda spec: spec.update(size=[2048, 2048]), "dimensions")

    def test_out_of_source_frame_is_rejected(self):
        self.rejected(lambda spec: spec["props"][0].update(crops=[[0, 0, 1255, 200]]), "outside")

    def test_incomplete_direction_set_is_rejected(self):
        self.rejected(lambda spec: spec["props"][1].update(crops=spec["props"][1]["crops"][:2]), "orientations")

    def test_per_prop_source_has_the_same_hash_fence(self):
        def edit(spec):
            spec["props"][0]["sheet"] = {
                "source": spec["source"], "sha256": "0" * 64, "size": spec["size"]
            }
        self.rejected(edit, "Source changed")

    def test_shared_scale_does_not_enlarge_a_narrow_view(self):
        from PIL import Image
        import hashlib

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sheet = Image.new("RGBA", (32, 16), (120, 80, 40, 255))
            source = root / "source.png"
            sheet.save(source)
            spec = {
                "source": "source.png", "sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
                "size": [32, 16], "label": "Directional test", "output": "pack.json",
                "props": [{"key": "desk", "label": "Desk", "side": 32,
                           "sharedScale": True,
                           "crops": [[0, 0, 32, 16], [0, 0, 8, 16],
                                     [0, 0, 32, 16], [0, 0, 8, 16]]}],
            }
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps(spec))
            _, payload = encoder.encode(manifest)
            frames = json.loads(payload)["props"][0]["frames"]
            widths = [max(sum(row[x:x+2] != "00" for x in range(0, len(row), 2))
                          for row in frame) for frame in frames]
            self.assertEqual(widths, [30, 8, 30, 8])


if __name__ == "__main__":
    unittest.main()
