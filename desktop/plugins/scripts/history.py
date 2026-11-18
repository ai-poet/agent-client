"""Read immutable package versions and release notes from committed Git history."""

from io import BytesIO
import json
import re
import subprocess


LOCALES = ("en", "zh-CN", "zh-TW", "ja", "ko", "fr", "de", "es", "pt-BR", "ru")
NUMBER = r"(?:0|[1-9][0-9]*)"
PREVIEW = r"(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
VERSION = re.compile(
    rf"({NUMBER})\.({NUMBER})\.({NUMBER})(?:-({PREVIEW}(?:\.{PREVIEW})*))?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
)


def precedence(version):
    match = VERSION.fullmatch(version) if isinstance(version, str) else None
    if match is None:
        raise ValueError("Package versions must be semantic versions")
    major, minor, patch, preview = match.groups()
    identifiers = tuple((0, int(part)) if part.isascii() and part.isdigit() else (1, part)
                        for part in preview.split(".")) if preview else ()
    return int(major), int(minor), int(patch), preview is None, identifiers


def section(text, version):
    if text is None:
        return None
    selected = False
    lines = []
    for line in text.splitlines(keepends=True):
        if line.rstrip().startswith("## "):
            if selected:
                break
            selected = line.rstrip()[3:].strip() == version
        elif selected:
            lines.append(line)
    text = "".join(lines).strip()
    if not text:
        return None
    if any((ord(char) < 32 and char not in "\n\r\t") or 127 <= ord(char) <= 159
           for char in text):
        raise ValueError("Release notes contain control characters")
    return text


def notes(read, version):
    text = section(read("CHANGELOG.md"), version)
    if text is None:
        return None
    locales = {}
    for locale in LOCALES:
        localized = section(read(f"CHANGELOG.{locale}.md"), version)
        if localized is not None:
            locales[locale] = localized
    result = {"text": text}
    if locales:
        result["locales"] = locales
    return result


def git(root, *arguments):
    return subprocess.check_output(["git", *arguments], cwd=root, text=True).strip()


def objects(root, names):
    if not names:
        return {}
    result = subprocess.run(["git", "cat-file", "--batch"], cwd=root, check=True,
                            input=("\n".join(names) + "\n").encode(), capture_output=True)
    output = BytesIO(result.stdout)
    contents = {}
    for name in names:
        header = output.readline().split()
        if len(header) == 2 and header[1] == b"missing":
            contents[name] = None
            continue
        if len(header) != 3 or header[1] != b"blob":
            raise ValueError("Catalog history requires file objects")
        size = int(header[2])
        content = output.read(size)
        if len(content) != size or output.read(1) != b"\n":
            raise ValueError("Incomplete package history object")
        contents[name] = content.decode("utf-8")
    return contents


def lineage(history, manifests, name):
    graph = {row[0]: row[1:] for row in history}
    parsed = {}

    def manifest(commit):
        if commit not in parsed:
            text = manifests.get(f"{commit}:{name}/plugin.json")
            data = json.loads(text) if text is not None else None
            if isinstance(data, dict) and data.get("name") == name:
                try:
                    parsed[commit] = data, precedence(data.get("version"))
                except ValueError:
                    parsed[commit] = None
            else:
                parsed[commit] = None
        return parsed[commit]

    if not history or manifest(history[0][0]) is None:
        raise ValueError("Current package history requires a valid manifest")
    pending = [history[0][0]]
    selected = set()
    while pending:
        commit = pending.pop()
        if commit in selected:
            continue
        _, current = manifest(commit)
        selected.add(commit)
        for parent in graph[commit]:
            previous = manifest(parent)
            # Legacy resets delimit discovery of previously unindexed package histories.
            if previous is not None and previous[1] <= current:
                pending.append(parent)
    return [(commit, manifest(commit)[0]) for commit in graph if commit in selected]


def trees(root, names):
    if not names:
        return {}
    result = subprocess.run(["git", "cat-file", "--batch-check"], cwd=root, check=True,
                            input="\n".join(names) + "\n", capture_output=True, text=True)
    contents = {}
    for name, line in zip(names, result.stdout.splitlines(), strict=True):
        fields = line.split()
        if len(fields) != 3 or fields[1] != "tree":
            raise ValueError("Published package commits require available package trees")
        contents[name] = fields[0]
    return contents


def recorded(root, packages, withdrawals):
    commits = git(root, "log", "--first-parent", "--format=%H", "--",
                  "catalog.json").splitlines()
    if not commits:
        return {}, None
    current = objects(root, ["HEAD:catalog.json"])["HEAD:catalog.json"]
    if current is None:
        raise ValueError("Restore the committed catalog before generating package history")
    selected = {(package["id"], record["version"]): record["commit"]
                for package in json.loads(current).get("packages", [])
                if package["id"] in packages for record in package.get("versions", [])}
    # Older generators removed withdrawn releases; recover only those explicit identities.
    pending = {(name, version) for name, versions in withdrawals.items()
               for version in versions if name in packages and (name, version) not in selected}
    for offset in range(0, len(commits), 16):
        if not pending:
            break
        snapshots = objects(root, [f"{commit}:catalog.json" for commit in commits[offset:offset + 16]])
        for text in snapshots.values():
            if text is None:
                continue
            for package in json.loads(text).get("packages", []):
                for record in package.get("versions", []):
                    key = package["id"], record["version"]
                    if key in pending:
                        selected[key] = record["commit"]
                        pending.remove(key)
    return selected, commits[0]


def versions(root, packages, withdrawals=None):
    if git(root, "rev-parse", "--is-shallow-repository") == "true":
        raise ValueError("Catalog history requires a complete Git checkout")
    if git(root, "status", "--porcelain", "--untracked-files=all", "--", *packages):
        raise ValueError("Commit package changes before generating catalog history")
    commits = {name: [line.split() for line in git(
        root, "rev-list", "--full-history", "--topo-order", "--parents",
        "HEAD", "--", name + "/").splitlines()]
               for name in packages}
    manifests = objects(root, [f"{row[0]}:{name}/plugin.json"
                               for name, history in commits.items() for row in history])
    histories = {name: lineage(history, manifests, name) for name, history in commits.items()}
    canonical, publication = recorded(root, packages, withdrawals or {})
    # The committed catalog owns legacy identities; newly introduced content must match them.
    published = set(git(root, "rev-list", publication).splitlines()) if publication else set()
    recent = {}
    for name, history in commits.items():
        recent[name] = []
        for commit, *_ in history:
            if commit in published:
                continue
            text = manifests[f"{commit}:{name}/plugin.json"]
            manifest = json.loads(text) if text is not None else None
            if isinstance(manifest, dict) and manifest.get("name") == name:
                precedence(manifest.get("version"))
                recent[name].append((commit, manifest))
        if publication is None:
            histories[name] = recent[name]
    references = {f"{commit}:{name}" for name, history in histories.items()
                  for commit, _ in [*history, *recent[name]]}
    references.update(f"{commit}:{name}" for (name, _), commit in canonical.items())
    contents = trees(root, sorted(references))
    result = {}
    for name, history in histories.items():
        candidates = {}
        for commit, manifest in [*history, *recent[name]]:
            candidates.setdefault(manifest["version"], set()).add(commit)
        selected = {version: commit for (package, version), commit in canonical.items()
                    if package == name}
        identities = {}
        for version in [*selected, *candidates]:
            identities.setdefault(precedence(version), set()).add(version)
        for version, entries in candidates.items():
            if version in selected:
                expected = contents[f"{selected[version]}:{name}"]
                checked = [commit for commit, manifest in recent[name] if manifest["version"] == version]
                if version == history[0][1]["version"]:
                    checked.append(history[0][0])
            else:
                if len(identities[precedence(version)]) > 1:
                    raise ValueError(f"Package {name} {version} reuses SemVer precedence; "
                                     "build metadata is not a version bump")
                # A new version has one content tree, even before its first publication.
                first = next(commit for commit, *_ in reversed(commits[name]) if commit in entries)
                selected[version] = first
                expected = contents[f"{first}:{name}"]
                checked = entries
            if any(contents[f"{commit}:{name}"] != expected for commit in checked):
                raise ValueError(f"Package {name} {version} changed content; increase the package version")
        records = []
        for version, commit in selected.items():
            source = f"{commit}:{name}/plugin.json"
            text = manifests.get(source)
            if text is None:
                text = objects(root, [source])[source]
            manifest = json.loads(text) if text is not None else None
            if not isinstance(manifest, dict) or manifest.get("name") != name or manifest.get("version") != version:
                raise ValueError("Published package commits must match their names and versions")
            record = {"version": version, "commit": commit}
            extensions = manifest.get("extensions", {})
            if "dev.sailry.platform" in extensions:
                extension = extensions["dev.sailry.platform"]
                if not isinstance(extension, dict) or not isinstance(extension.get("api_version"), str):
                    raise ValueError("Sailry extension API versions must be strings")
                record["api_version"] = extension["api_version"]
                if "min_sailry_version" in extension:
                    minimum = extension["min_sailry_version"]
                    precedence(minimum)
                    record["min_sailry_version"] = minimum
            records.append(record)
        order = {row[0]: index for index, row in enumerate(commits[name])}
        result[name] = sorted(records, key=lambda record: (
            precedence(record["version"]), -order.get(record["commit"], len(order))), reverse=True)
    filenames = ["CHANGELOG.md", *(f"CHANGELOG.{locale}.md" for locale in LOCALES)]
    snapshots = objects(root, [f"{record['commit']}:{name}/{filename}"
                               for name, records in result.items() for record in records
                               for filename in filenames])
    for name, records in result.items():
        for record in records:
            prefix = f"{record['commit']}:{name}/"
            release_notes = notes(lambda filename: snapshots[prefix + filename], record["version"])
            if release_notes:
                record["release_notes"] = release_notes
    return result
