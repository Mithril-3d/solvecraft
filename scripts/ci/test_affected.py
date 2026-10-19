#!/usr/bin/env python3
"""Self-test for affected.py. Run: python3 -I scripts/ci/test_affected.py (from the repo root).

Uses a small synthetic workspace, then the real one (`cargo metadata --no-deps`).
"""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import affected  # noqa: E402

R = "/ws"


def pkg(name, dir_, deps=()):
    return {
        "name": name,
        "manifest_path": f"{R}/{dir_}/Cargo.toml",
        "dependencies": [{"name": d, "path": f"{R}/{p}"} for d, p in deps] + [{"name": "serde"}],
    }


SYNTH = {
    "workspace_root": R,
    "packages": [
        pkg("base", "crates/base"),
        pkg("mid", "crates/mid", [("base", "crates/base")]),
        pkg("top", "apps/top", [("mid", "crates/mid")]),
        pkg("alone", "crates/alone"),
    ],
}


class Synthetic(unittest.TestCase):
    def test_leaf_selects_only_itself(self):
        self.assertEqual(affected.select(SYNTH, ["apps/top/src/main.rs"]), (False, ["top"]))

    def test_reverse_dependencies_are_added(self):
        self.assertEqual(affected.select(SYNTH, ["crates/base/src/lib.rs"]), (False, ["base", "mid", "top"]))

    def test_unrelated_package_is_not_selected(self):
        full, pk = affected.select(SYNTH, ["crates/mid/Cargo.toml"])
        self.assertEqual((full, pk), (False, ["mid", "top"]))

    def test_full_triggers(self):
        for p in ["Cargo.lock", "Cargo.toml", ".cargo/config.toml", ".config/nextest.toml", "rust-toolchain.toml",
                  ".github/workflows/ci.yml", "vendor/x/lib.rs", "tests/oracle/a.json", "assets/a.png",
                  "scripts/ci/affected.py", "unknown.txt"]:
            self.assertEqual(affected.select(SYNTH, ["crates/alone/a.rs", p]), (True, []), p)

    def test_docs_only_selects_nothing(self):
        self.assertEqual(affected.select(SYNTH, ["docs/book/a.md", "README.md", "packaging/x.yml"]), (False, []))


class RealWorkspace(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.meta = affected.load_metadata(None)

    def test_render_change_selects_render_and_dependents_not_kernel(self):
        full, pk = affected.select(self.meta, ["crates/render/src/lib.rs"])
        self.assertFalse(full)
        self.assertIn("solidcraft-render", pk)
        self.assertGreater(len(pk), 1)
        self.assertNotIn("solidcraft-kernel", pk)

    def test_cargo_lock_selects_full(self):
        self.assertEqual(affected.select(self.meta, ["Cargo.lock"]), (True, []))

    def test_docs_only_selects_nothing(self):
        self.assertEqual(affected.select(self.meta, ["docs/book/src/dev/testing.md", "CONTRIBUTING.md"]), (False, []))


if __name__ == "__main__":
    unittest.main()
