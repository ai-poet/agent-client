"""The official index preserves package metadata for installs and update checks."""

import importlib.util
from pathlib import Path
import json
import os
import tempfile
import subprocess
import sys
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "catalog.py"
sys.path.insert(0, str(SCRIPT.parent))
SPEC = importlib.util.spec_from_file_location("catalog", SCRIPT)
catalog = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(catalog)


def manifest(root, name, **metadata):
    directory = root / name
    directory.mkdir()
    data = {"name": name, "version": "1.10.0", **metadata}
    (directory / "plugin.json").write_text(json.dumps(data), encoding="utf-8")
    return data


class Catalog(unittest.TestCase):
    def test_preserves_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data = manifest(root, "sample", description="Package description", extensions={
                "dev.sailry.platform": {"description": {
                    "label": "Package description", "locales": {"zh-CN": "插件描述"},
                }},
            })
            self.assertEqual(catalog.catalog(root, version_history={}), {
                "version": 1, "packages": [{"id": "sample", "manifest": data,
                                          "provides_models": False, "permissions": [], "versions": []}], "skills": [],
            })

    def test_derives_model_summary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data = manifest(root, "sample", extensions={
                "dev.sailry.platform": {
                    "model_providers": [{"id": "models"}],
                    "actions": ["models.provide", "credentials.inspect"],
                },
            })
            entry = catalog.catalog(root, version_history={})["packages"][0]
            self.assertEqual(entry["manifest"], data)
            self.assertTrue(entry["provides_models"])
            self.assertEqual(entry["permissions"], ["models.provide", "credentials.inspect"])

    def test_orders_packages(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest(root, "second")
            manifest(root, "first")
            nested = root / "examples"
            nested.mkdir()
            manifest(nested, "optional")
            self.assertEqual(
                [package["id"] for package in catalog.catalog(root, version_history={})["packages"]],
                ["first", "second"],
            )

    def test_selects_exact_release_notes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest(root, "sample")
            package = root / "sample"
            (package / "CHANGELOG.md").write_text("## 2.0.0\n- Future\n\n## 1.10.0\n- Current\n\n## 1.9.0\n- Previous\n")
            (package / "CHANGELOG.zh-CN.md").write_text("## 1.10.0\n- 本次变更\n", encoding="utf-8")
            self.assertEqual(catalog.catalog(root, version_history={})["packages"][0]["release_notes"],
                             {"text": "- Current", "locales": {"zh-CN": "- 本次变更"}})
            (package / "CHANGELOG.md").write_text("## 1.9.0\n- Previous\n")
            self.assertNotIn("release_notes", catalog.catalog(root, version_history={})["packages"][0])

    def test_reports_stale_existing_translations(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest(root, "sample")
            package = root / "sample"
            (package / "CHANGELOG.md").write_text("## 1.10.0\n- Current\n")
            (package / "CHANGELOG.zh-CN.md").write_text("## 1.9.0\n- 旧版说明\n", encoding="utf-8")
            (package / "CHANGELOG.ja.md").write_text("## 1.10.0\n- 現在の変更\n", encoding="utf-8")
            self.assertEqual(catalog.stale_notes(root), ["sample 1.10.0: zh-CN"])
            self.assertEqual(catalog.catalog(root, version_history={})["packages"][0]["release_notes"],
                             {"text": "- Current", "locales": {"ja": "- 現在の変更"}})

    def test_preserves_real_skill_frontmatter(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest(root, "sample")
            skill = root / "sample" / "skills" / "research"
            skill.mkdir(parents=True)
            frontmatter = "---\nname: research\ndescription: Research a topic\nmetadata:\n  display-name.zh-CN: 研究\n---\n"
            (skill / "SKILL.md").write_text(frontmatter + "Private body is not a catalog field\n", encoding="utf-8")
            self.assertEqual(catalog.catalog(root, version_history={})["skills"], [{"package": "sample",
                "path": "skills/research/SKILL.md", "frontmatter": frontmatter}])

    def test_rejects_directory_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest(root, "sample")
            path = root / "sample" / "plugin.json"
            path.write_text(json.dumps({"name": "different"}), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "name must match"):
                catalog.catalog(root, version_history={})


def git(root, *arguments):
    return subprocess.check_output(["git", *arguments], cwd=root, text=True,
                                   stderr=subprocess.PIPE, env={**os.environ,
                                       "GIT_AUTHOR_DATE": "2000-01-01T00:00:00+00:00",
                                       "GIT_COMMITTER_DATE": "2000-01-01T00:00:00+00:00",
                                   }).strip()


def repository(root):
    git(root, "init", "--quiet", "--initial-branch=main")
    git(root, "config", "user.name", "Fixture")
    git(root, "config", "user.email", "fixture@example.invalid")
    git(root, "config", "commit.gpgsign", "false")
    git(root, "config", "core.hooksPath", os.devnull)


def commit(root, message):
    git(root, "add", ".")
    git(root, "commit", "--quiet", "-m", message)
    return git(root, "rev-parse", "HEAD")


def release(root, version, minimum=None, text=None, localized=None, api="v1", platform=True):
    package = root / "sample"
    package.mkdir(exist_ok=True)
    extension = {"api_version": api} if api is not None else {}
    if minimum is not None:
        extension["min_sailry_version"] = minimum
    data = {"name": "sample", "version": version}
    if platform:
        data["extensions"] = {"dev.sailry.platform": extension}
    (package / "plugin.json").write_text(json.dumps(data), encoding="utf-8")
    if text is not None:
        (package / "CHANGELOG.md").write_text(text, encoding="utf-8")
    if localized is not None:
        (package / "CHANGELOG.zh-CN.md").write_text(localized, encoding="utf-8")
    return commit(root, f"Release {version}")


def publish(root, records=None):
    data = catalog.catalog(root) if records is None else {
        "version": 1, "packages": [{"id": "sample", "versions": records}],
    }
    (root / "catalog.json").write_text(json.dumps(data), encoding="utf-8")
    commit(root, "Publish catalog")
    return data


class History(unittest.TestCase):
    def test_uses_first_matching_content_commit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            first = release(root, "1.0.0")
            second = release(root, "1.1.0")
            git(root, "checkout", first, "--", "sample")
            restored = commit(root, "Restore the original package tree")
            self.assertNotEqual(first, restored)
            records = catalog.catalog(root)["packages"][0]["versions"]
            self.assertEqual([(record["version"], record["commit"]) for record in records],
                             [("1.1.0", second), ("1.0.0", first)])

    def test_preserves_recorded_content(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            release(root, "1.0.0")
            publish(root)
            (root / "sample" / "main.js").write_text("export const value = 1;\n")
            final = commit(root, "Record legacy same-version content")
            publish(root, [{"version": "1.0.0", "commit": final}])
            before = catalog.catalog(root)
            (root / "README.md").write_text("Repository documentation\n")
            commit(root, "Update repository documentation")
            self.assertEqual(catalog.catalog(root), before)
            self.assertEqual(before["packages"][0]["versions"],
                             [{"version": "1.0.0", "commit": final, "api_version": "v1"}])
            (root / "catalog.json").write_text("{}")
            self.assertEqual(catalog.catalog(root), before)

    def test_rejects_unpublished_content_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            release(root, "1.0.0")
            (root / "sample" / "main.js").write_text("export const value = 1;\n")
            commit(root, "Change content without increasing version")
            with self.assertRaisesRegex(ValueError, "sample 1.0.0 changed content"):
                catalog.catalog(root)
            release(root, "1.1.0")
            with self.assertRaisesRegex(ValueError, "sample 1.0.0 changed content"):
                catalog.catalog(root)

    def test_rejects_published_content_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            release(root, "1.0.0")
            publish(root)
            (root / "sample" / "main.js").write_text("export const value = 1;\n")
            commit(root, "Change published content")
            with self.assertRaisesRegex(ValueError, "sample 1.0.0 changed content"):
                catalog.catalog(root)
            release(root, "1.1.0")
            with self.assertRaisesRegex(ValueError, "sample 1.0.0 changed content"):
                catalog.catalog(root)

    def test_preserves_immutable_notes_and_minimums(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            first = release(root, "0.1.0-alpha.4", "0.1.0-alpha.4",
                            "## 0.1.0-alpha.4\n- Preview\n",
                            "## 0.1.0-alpha.4\n- 预览\n")
            latest = release(root, "0.1.0", "0.1.0-alpha.5",
                             "## 0.1.0\n- Stable\n\n## 0.1.0-alpha.4\n- Rewritten later\n")
            entry = catalog.catalog(root)["packages"][0]
            self.assertEqual(entry["manifest"]["version"], "0.1.0")
            self.assertEqual(entry["release_notes"], {"text": "- Stable"})
            self.assertEqual(entry["versions"], [
                {"version": "0.1.0", "commit": latest, "api_version": "v1", "min_sailry_version": "0.1.0-alpha.5",
                 "release_notes": {"text": "- Stable"}},
                {"version": "0.1.0-alpha.4", "commit": first, "api_version": "v1",
                 "min_sailry_version": "0.1.0-alpha.4",
                 "release_notes": {"text": "- Preview", "locales": {"zh-CN": "- 预览"}}},
            ])

    def test_preserves_unknown_and_absent_api(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            plain = release(root, "1.0.0", platform=False)
            future = release(root, "2.0.0", "0.2.0-alpha.1", "## 2.0.0\n- Future API\n", api="v99")
            records = catalog.catalog(root)["packages"][0]["versions"]
            self.assertEqual(records, [
                {"version": "2.0.0", "commit": future, "api_version": "v99",
                 "min_sailry_version": "0.2.0-alpha.1", "release_notes": {"text": "- Future API"}},
                {"version": "1.0.0", "commit": plain},
            ])

    def test_requires_string_api(self):
        with tempfile.TemporaryDirectory() as directory:
            for index, api in enumerate((None, 1)):
                with self.subTest(api=api):
                    root = Path(directory) / str(index)
                    root.mkdir()
                    repository(root)
                    release(root, "1.0.0", api=api)
                    with self.assertRaisesRegex(ValueError, "API versions must be strings"):
                        catalog.catalog(root)

    def test_orders_recorded_build_ties(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            versions = ["0.1.0-alpha.4", "0.1.0-alpha.5", "0.1.0", "0.1.0+build.1",
                        "0.1.0+build.2", "0.2.0-alpha.1"]
            releases = [{"version": version, "commit": release(root, version)} for version in versions]
            publish(root, releases)
            self.assertEqual([item["version"] for item in catalog.catalog(root)["packages"][0]["versions"]],
                             list(reversed(versions)))

    def test_rejects_new_build_aliases(self):
        with tempfile.TemporaryDirectory() as directory:
            for original in ("1.0.0", "1.0.0+build.1"):
                for published in (False, True):
                    with self.subTest(original=original, published=published):
                        root = Path(directory) / f"{original}-{published}"
                        root.mkdir()
                        repository(root)
                        release(root, original)
                        if published:
                            publish(root)
                        (root / "sample" / "main.js").write_text("export const changed = true;\n")
                        release(root, "1.0.0+build.2")
                        with self.assertRaisesRegex(ValueError, "reuses SemVer precedence"):
                            catalog.catalog(root)

    def test_accepts_new_precedence_with_build(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            release(root, "0.9.0")
            publish(root)
            latest = release(root, "1.0.0+build.2")
            records = catalog.catalog(root)["packages"][0]["versions"]
            self.assertEqual(records[0], {"version": "1.0.0+build.2", "commit": latest, "api_version": "v1"})

    def test_keeps_merged_version_commits(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            release(root, "1.0.0")
            git(root, "switch", "--quiet", "-c", "feature")
            second = release(root, "2.0.0")
            git(root, "switch", "--quiet", "main")
            first = release(root, "1.1.0")
            result = subprocess.run(["git", "merge", "--no-ff", "feature"], cwd=root,
                                    capture_output=True)
            self.assertEqual(result.returncode, 1)
            merged = release(root, "3.0.0")
            records = catalog.catalog(root)["packages"][0]["versions"]
            self.assertEqual([(item["version"], item["commit"]) for item in records[:3]],
                             [("3.0.0", merged), ("2.0.0", second), ("1.1.0", first)])

    def test_preserves_same_tree_merge_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            release(root, "1.0.0")
            publish(root)
            git(root, "switch", "--quiet", "-c", "feature")
            (root / "README.md").write_text("Feature documentation\n")
            commit(root, "Document feature")
            alternative = release(root, "1.1.0")
            git(root, "switch", "--quiet", "main")
            canonical = release(root, "1.1.0")
            self.assertNotEqual(canonical, alternative)
            before = publish(root)
            git(root, "merge", "--no-ff", "--no-edit", "feature")
            records = catalog.catalog(root)["packages"][0]["versions"]
            self.assertEqual(records, before["packages"][0]["versions"])
            self.assertEqual(records[0]["commit"], canonical)

    def test_rejects_merged_version_collisions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            release(root, "1.0.0")
            publish(root)
            git(root, "switch", "--quiet", "-c", "feature")
            release(root, "1.1.0", text="## 1.1.0\n- Branch content\n")
            git(root, "switch", "--quiet", "main")
            release(root, "1.1.0", text="## 1.1.0\n- Published content\n")
            publish(root)
            result = subprocess.run(["git", "merge", "--no-ff", "--no-edit", "feature"], cwd=root,
                                    capture_output=True)
            self.assertEqual(result.returncode, 1)
            release(root, "1.2.0", text="## 1.2.0\n- Combined changes\n")
            with self.assertRaisesRegex(ValueError, "sample 1.1.0 changed content"):
                catalog.catalog(root)

    def test_rejects_uncommitted_package_content(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            release(root, "1.0.0")
            (root / "sample" / "main.js").write_text("export const pending = true;\n")
            with self.assertRaisesRegex(ValueError, "Commit package changes"):
                catalog.catalog(root)

    def test_preserves_recorded_version_reset(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            release(root, "0.1.0", text="## 0.1.0\n- Retired content\n")
            release(root, "0.3.2")
            reset = release(root, "0.1.0", text="## 0.1.0\n- Current lineage\n")
            latest = release(root, "0.1.1")
            publish(root, [{"version": "0.1.1", "commit": latest}, {"version": "0.1.0", "commit": reset}])
            records = catalog.catalog(root)["packages"][0]["versions"]
            self.assertEqual([(item["version"], item["commit"]) for item in records],
                             [("0.1.1", latest), ("0.1.0", reset)])
            self.assertEqual(records[1]["release_notes"], {"text": "- Current lineage"})

    def test_rejects_new_version_reset(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            release(root, "0.1.0", text="## 0.1.0\n- Published content\n")
            publish(root)
            release(root, "0.3.2")
            release(root, "0.1.0", text="## 0.1.0\n- Reset content\n")
            with self.assertRaisesRegex(ValueError, "sample 0.1.0 changed content"):
                catalog.catalog(root)

    def test_withdraws_incompatible_historical_version(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            compatible = release(root, "0.1.25", "0.1.2")
            withdrawn = release(root, "0.1.26", "0.1.2")
            latest = release(root, "0.1.27", "0.1.3")
            (root / "catalog-withdrawn.json").write_text('{"sample":["0.1.26"]}')
            commit(root, "Withdraw incompatible release")
            records = catalog.catalog(root)["packages"][0]["versions"]
            self.assertEqual([(record["version"], record["commit"]) for record in records],
                             [("0.1.27", latest), ("0.1.26", withdrawn), ("0.1.25", compatible)])
            self.assertEqual([record["min_sailry_version"] for record in records],
                             ["0.1.3", "0.1.2", "0.1.2"])
            self.assertNotIn("withdrawn", records[0])
            self.assertTrue(records[1]["withdrawn"])
            self.assertNotIn("withdrawn", records[2])
            (root / "catalog-withdrawn.json").write_text('{"sample":["0.1.26","0.1.27"]}')
            latest_withdrawn = catalog.catalog(root)["packages"][0]["versions"]
            self.assertTrue(latest_withdrawn[0]["withdrawn"])
            self.assertEqual(latest_withdrawn[0]["commit"], latest)
            for invalid in ('{"sample":["0.1.26","0.1.26"]}', '{"sample":[1]}',
                            '{"sample":["0.1.24"]}', '{"other":["0.1.26"]}'):
                (root / "catalog-withdrawn.json").write_text(invalid)
                with self.assertRaisesRegex(ValueError, "Invalid catalog withdrawal"):
                    catalog.catalog(root)

    def test_recovers_withdrawn_recorded_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository(root)
            first = release(root, "1.0.0", "0.1.0", "## 1.0.0\n- Original notes\n")
            publish(root)
            release(root, "1.0.0", "0.1.1", "## 1.0.0\n- Legacy changed notes\n")
            latest = release(root, "1.1.0")
            (root / "catalog-withdrawn.json").write_text('{"sample":["1.0.0"]}')
            publish(root, [{"version": "1.1.0", "commit": latest}])
            records = catalog.catalog(root)["packages"][0]["versions"]
            self.assertEqual(records[1], {
                "version": "1.0.0", "commit": first, "api_version": "v1",
                "min_sailry_version": "0.1.0", "release_notes": {"text": "- Original notes"},
                "withdrawn": True,
            })
            publish(root)
            self.assertEqual(catalog.catalog(root)["packages"][0]["versions"], records)

    def test_rejects_incomplete_checkouts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "source"
            root.mkdir()
            repository(root)
            release(root, "1.0.0")
            release(root, "2.0.0")
            checkout = Path(directory) / "checkout"
            git(root, "clone", "--quiet", "--depth", "1", root.resolve().as_uri(), str(checkout))
            with self.assertRaisesRegex(ValueError, "complete Git checkout"):
                catalog.catalog(checkout)


if __name__ == "__main__":
    unittest.main()
