"""Index fixtures do not require a Sailry build, native UI, or image decoder."""

import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "catalog.py"
SPEC = importlib.util.spec_from_file_location("theme_catalog", SCRIPT)
catalog = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(catalog)


def manifest(identity="fixture"):
    return {
        "version": 1, "id": identity, "name": "Fixture", "package_version": "0.1.0",
        "description": "Fixture description", "homepage": "https://sailry.dev",
        "license": {"author": "Sailry", "name": "Apache-2.0", "text": "Fixture license"},
        "light": {"theme": {"mode": "light"}, "assets": {
            "background.window": {"path": "assets/background.png"}}},
        "dark": {"theme": {"mode": "dark"}, "assets": {
            "background.window": {"path": "assets/background.png"}}},
    }


class Catalog(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.package = self.root / "fixture"
        (self.package / "assets").mkdir(parents=True)
        (self.package / "assets/background.png").write_bytes(b"fixture resource")
        self.manifest = manifest()
        self.write()

    def write(self):
        (self.package / "theme.json").write_text(json.dumps(self.manifest), encoding="utf-8")

    def test_indexes_manifest_metadata(self):
        result = catalog.catalog(self.root)
        self.assertEqual(result["version"], 1)
        self.assertEqual(len(result["themes"]), 1)
        entry = result["themes"][0]
        self.assertEqual({key: value for key, value in entry.items()
                          if key not in {"digest", "resources", "modes", "previews"}}, {
            "id": "fixture", "path": "fixture", "name": "Fixture", "package_version": "0.1.0",
            "author": "Sailry", "license": "Apache-2.0", "description": "Fixture description",
            "homepage": "https://sailry.dev",
        })
        self.assertRegex(entry["digest"], r"^sha256:[a-f0-9]{64}$")

    def test_indexes_exact_package_resources(self):
        entry = catalog.catalog(self.root)["themes"][0]
        files = {resource["path"]: (self.package / resource["path"]).read_bytes()
                 for resource in entry["resources"]}
        self.assertEqual(list(files), ["assets/background.png", "theme.json"])
        self.assertEqual(entry["digest"], catalog.digest(files))
        for resource in entry["resources"]:
            encoded = files[resource["path"]]
            self.assertEqual(resource["bytes"], len(encoded))
            self.assertEqual(resource["digest"],
                             "sha256:" + catalog.hashlib.sha256(encoded).hexdigest())
        self.assertEqual(entry["modes"], ["light", "dark"])
        self.assertEqual(entry["previews"], {
            "light": "assets/background.png", "dark": "assets/background.png"})

    def test_keeps_market_metadata_outside_packages(self):
        initial = catalog.catalog(self.root)["themes"][0]
        (self.package / "market.json").write_text(json.dumps({
            "description_locales": {"zh-CN": "主题介绍", "fr": "Description du thème"}
        }), encoding="utf-8")
        entry = catalog.catalog(self.root)["themes"][0]
        self.assertEqual(entry["description_locales"]["zh-CN"], "主题介绍")
        self.assertEqual(entry["digest"], initial["digest"])
        self.assertEqual(entry["resources"], initial["resources"])

    def test_rejects_invalid_market_metadata(self):
        for value in ([], {"archive": "https://example.com/theme.zip"},
                      {"description_locales": []}, {"description_locales": {"unknown": "text"}},
                      {"description_locales": {"en": ""}}):
            with self.subTest(value=value):
                (self.package / "market.json").write_text(json.dumps(value), encoding="utf-8")
                with self.assertRaises(ValueError):
                    catalog.catalog(self.root)

    def test_sorts_packages_by_id(self):
        second = self.root / "alpha"
        second.mkdir()
        value = manifest("alpha")
        for mode in ("light", "dark"):
            del value[mode]["assets"]
        (second / "theme.json").write_text(json.dumps(value), encoding="utf-8")
        self.assertEqual([entry["id"] for entry in catalog.catalog(self.root)["themes"]],
                         ["alpha", "fixture"])

    def test_resources_change_digest(self):
        initial = catalog.catalog(self.root)
        self.assertEqual(initial, catalog.catalog(self.root))
        (self.package / "assets/background.png").write_bytes(b"changed resource")
        changed = catalog.catalog(self.root)
        self.assertNotEqual(initial["themes"][0]["digest"], changed["themes"][0]["digest"])
        self.assertEqual(initial["themes"][0]["package_version"], changed["themes"][0]["package_version"])

    def test_unreferenced_files_do_not_change_digest(self):
        initial = catalog.catalog(self.root)
        (self.package / "README.md").write_text("Updated package guide", encoding="utf-8")
        (self.package / "assets/unused.png").write_bytes(b"unused resource")
        self.assertEqual(initial, catalog.catalog(self.root))

    def test_keeps_source_files_unchanged(self):
        before = {path.relative_to(self.package): path.read_bytes()
                  for path in self.package.rglob("*") if path.is_file()}
        catalog.generate(self.root)
        catalog.generate(self.root, check=True)
        self.assertEqual(before, {path.relative_to(self.package): path.read_bytes()
                                for path in self.package.rglob("*") if path.is_file()})

    def test_check_rejects_stale_index_without_writing(self):
        catalog.generate(self.root)
        target = self.root / "catalog.json"
        initial = target.read_bytes()
        self.manifest["name"] = "Updated name"
        self.write()
        with self.assertRaisesRegex(ValueError, "Catalog is stale"):
            catalog.generate(self.root, check=True)
        self.assertEqual(target.read_bytes(), initial)

    def test_accepts_semantic_versions(self):
        for version in ("0.1.1", "1.0.0", "1.2.3-rc.1+build.007"):
            with self.subTest(version=version):
                self.manifest["package_version"] = version
                self.write()
                self.assertEqual(catalog.catalog(self.root)["themes"][0]["package_version"], version)

    def test_rejects_invalid_semantic_versions(self):
        for version in (1, None, "1", "1.2", "v1.2.3", "01.2.3", "1.2.3-01",
                        "1.2.3-", "1.2.3+", "1.2.3-alpha..1", "1.2.3\n",
                        "18446744073709551616.0.0"):
            with self.subTest(version=version):
                self.manifest["package_version"] = version
                self.write()
                with self.assertRaisesRegex(ValueError, "package version"):
                    catalog.catalog(self.root)

    def test_compares_precedence_without_build_metadata(self):
        ordered = ["1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta",
                   "1.0.0-beta.2", "1.0.0-beta.11", "1.0.0-rc.1", "1.0.0", "1.0.1"]
        for previous, candidate in zip(ordered, ordered[1:]):
            self.assertGreater(catalog.precedence(candidate), catalog.precedence(previous))
        self.assertEqual(catalog.precedence("0.1.0+first"), catalog.precedence("0.1.0+second"))
        self.assertEqual(catalog.precedence("0.1.0"), catalog.precedence("0.1.0+build"))
        self.assertLess(catalog.precedence("0.0.9"), catalog.precedence("0.1.0"))
        initial = catalog.catalog(self.root)["themes"][0]
        (self.package / "assets/background.png").write_bytes(b"changed resource")
        changed = catalog.catalog(self.root)["themes"][0]
        self.assertNotEqual(initial["digest"], changed["digest"])
        self.assertEqual(catalog.precedence(initial["package_version"]),
                         catalog.precedence(changed["package_version"]))

    def test_rejects_invalid_format_and_identity(self):
        for key, value in (("version", 2), ("version", True), ("id", "other"),
                           ("id", "../escape"), ("id", "Uppercase")):
            with self.subTest(key=key, value=value):
                self.manifest = manifest()
                self.manifest[key] = value
                self.write()
                with self.assertRaises(ValueError):
                    catalog.catalog(self.root)

    def test_rejects_unsafe_resource_paths(self):
        for path in ("../outside.png", "assets/../outside.png", "/assets/image.png",
                     "https://example.com/image.png", "assets\\image.png", "assets//image.png",
                     "assets/image.svg", "assets/image.png?query"):
            with self.subTest(path=path):
                self.manifest["light"]["assets"]["background.window"]["path"] = path
                self.write()
                with self.assertRaisesRegex(ValueError, "local raster paths"):
                    catalog.catalog(self.root)

    def test_rejects_missing_resource(self):
        (self.package / "assets/background.png").unlink()
        with self.assertRaisesRegex(ValueError, "resource is missing"):
            catalog.catalog(self.root)

    def test_rejects_symbolic_links(self):
        resource = self.package / "assets/background.png"
        external = self.root / "outside.png"
        external.write_bytes(resource.read_bytes())
        resource.unlink()
        resource.symlink_to(external)
        with self.assertRaisesRegex(ValueError, "symbolic links"):
            catalog.catalog(self.root)
        resource.unlink()
        resource.write_bytes(external.read_bytes())
        original = self.root / "original"
        self.package.rename(original)
        self.package.symlink_to(original, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symbolic links"):
            catalog.catalog(self.root)

    def test_rejects_resource_budget_overflow(self):
        with mock.patch.object(catalog, "FILE_LIMIT", 1):
            with self.assertRaisesRegex(ValueError, "too large"):
                catalog.catalog(self.root)
        with mock.patch.object(catalog, "PACKAGE_LIMIT", 1):
            with self.assertRaisesRegex(ValueError, "package limit"):
                catalog.catalog(self.root)

    def test_rejects_invalid_variants_and_metadata(self):
        fixtures = []
        for key, value in (("name", ""), ("license", {}), ("light", []),
                           ("dark", {"theme": {}, "assets": []}),
                           ("homepage", "https://user:secret@example.com")):
            sample = copy.deepcopy(manifest())
            sample[key] = value
            fixtures.append(sample)
        for sample in fixtures:
            with self.subTest(sample=sample):
                self.manifest = sample
                self.write()
                with self.assertRaises(ValueError):
                    catalog.catalog(self.root)


if __name__ == "__main__":
    unittest.main()
