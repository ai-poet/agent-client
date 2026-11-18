"""Generate the official v1 theme index from local manifests and resources."""

import argparse
import hashlib
import json
from pathlib import Path
import re
from urllib.parse import urlsplit


ROOT = Path(__file__).resolve().parents[1]
MANIFEST_LIMIT = 256 * 1024
FILE_LIMIT = 8 * 1024 * 1024
PACKAGE_LIMIT = 32 * 1024 * 1024
LOCALES = {"en", "zh-CN", "zh-TW", "ja", "ko", "fr", "de", "es", "pt-BR", "ru"}



def precedence(value):
    if not isinstance(value, str):
        raise ValueError("Theme package version must be valid SemVer")
    match = re.fullmatch(
        r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
        r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
        r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?", value)
    if not match:
        raise ValueError("Theme package version must be valid SemVer")
    raw_core = tuple(match[index] for index in (1, 2, 3))
    maximum = "18446744073709551615"
    if any(len(number) > len(maximum)
           or len(number) == len(maximum) and number > maximum for number in raw_core):
        raise ValueError("Theme package version exceeds the supported numeric domain")
    core = tuple(int(number) for number in raw_core)
    identifiers = match[4].split(".") if match[4] else []
    if any(part.isdecimal() and len(part) > 1 and part.startswith("0")
           for part in identifiers):
        raise ValueError("Theme package version must be valid SemVer")
    prerelease = tuple((0, len(part), part) if part.isdecimal() else (1, 0, part)
                       for part in identifiers)
    # Build metadata is deliberately absent from precedence.
    return (*core, not identifiers, prerelease)

def text(value, label, multiline=False):
    if not isinstance(value, str) or not value.strip() or any(
            ord(char) < 32 and (not multiline or char not in "\n\r\t") for char in value):
        raise ValueError(f"Invalid theme {label}")
    return value


def read(root, relative, limit):
    path = root
    if root.is_symlink():
        raise ValueError("Theme directories cannot use symbolic links")
    for part in relative.split("/"):
        path /= part
        if path.is_symlink():
            raise ValueError("Theme resources cannot use symbolic links")
    if not path.is_file() or path.stat().st_size > limit:
        raise ValueError(f"Theme resource is missing or too large: {relative}")
    data = path.read_bytes()
    if len(data) > limit:
        raise ValueError(f"Theme resource is too large: {relative}")
    return data


def resources(root, manifest, encoded):
    files = {"theme.json": encoded}
    for mode in ("light", "dark"):
        variant = manifest.get(mode)
        if not isinstance(variant, dict) or not isinstance(variant.get("theme"), dict):
            raise ValueError(f"Theme requires a {mode} variant")
        assets = variant.get("assets", {})
        if not isinstance(assets, dict):
            raise ValueError(f"Invalid {mode} theme resources")
        for asset in assets.values():
            path = asset.get("path") if isinstance(asset, dict) else None
            if (not isinstance(path, str) or not path.startswith("assets/")
                    or len(path.encode("utf-8")) > 512
                    or any(char in path for char in "\\:?#")
                    or any(ord(char) < 32 for char in path)
                    or any(part in ("", ".", "..") for part in path.split("/"))
                    or Path(path).suffix.lower() not in {".png", ".jpg", ".jpeg", ".webp", ".gif"}):
                raise ValueError("Theme resources must be local raster paths under assets/")
            if path not in files:
                files[path] = read(root, path, FILE_LIMIT)
    if sum(map(len, files.values())) > PACKAGE_LIMIT:
        raise ValueError("Theme resources exceed the package limit")
    return files


def digest(files):
    result = hashlib.sha256(b"Sailry theme package v1\0")
    for relative, encoded in sorted(files.items()):
        path = relative.encode("utf-8")
        result.update(len(path).to_bytes(8, "big"))
        result.update(path)
        result.update(len(encoded).to_bytes(8, "big"))
        result.update(encoded)
    return "sha256:" + result.hexdigest()


def package(root):
    encoded = read(root, "theme.json", MANIFEST_LIMIT)
    manifest = json.loads(encoded)
    if (not isinstance(manifest, dict) or type(manifest.get("version")) is not int
            or manifest["version"] != 1):
        raise ValueError("Theme manifest format must be 1")
    identity = manifest.get("id")
    if (not isinstance(identity, str) or not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", identity)
            or identity == "builtin" or identity != root.name):
        raise ValueError("Theme ID must match its directory")
    precedence(manifest.get("package_version"))
    license = manifest.get("license")
    if not isinstance(license, dict):
        raise ValueError("Theme license metadata is missing")
    entry = {
        "id": identity,
        "path": identity,
        "name": text(manifest.get("name"), "name"),
        "package_version": manifest["package_version"],
        "author": text(license.get("author"), "author"),
        "license": text(license.get("name"), "license name"),
    }
    text(license.get("text"), "license text", multiline=True)
    if "description" in manifest:
        entry["description"] = text(manifest["description"], "description", multiline=True)
    if "homepage" in manifest:
        homepage = text(manifest["homepage"], "homepage")
        parsed = urlsplit(homepage)
        if (parsed.scheme not in {"http", "https"} or not parsed.hostname
                or parsed.username is not None or parsed.password is not None
                or any(char.isspace() for char in homepage)):
            raise ValueError("Theme homepage must be an HTTP(S) URL without credentials")
        entry["homepage"] = homepage
    files = resources(root, manifest, encoded)
    entry["digest"] = digest(files)
    entry["resources"] = [
        {"path": relative, "bytes": len(data),
         "digest": "sha256:" + hashlib.sha256(data).hexdigest()}
        for relative, data in sorted(files.items())
    ]
    entry["modes"] = ["light", "dark"]
    entry["previews"] = {
        mode: manifest[mode]["assets"]["background.window"]["path"]
        for mode in ("light", "dark")
        if "background.window" in manifest[mode].get("assets", {})
    }
    if (root / "market.json").exists() or (root / "market.json").is_symlink():
        metadata = json.loads(read(root, "market.json", MANIFEST_LIMIT))
        if not isinstance(metadata, dict) or set(metadata) - {"description_locales"}:
            raise ValueError("Invalid theme market metadata")
        locales = metadata.get("description_locales", {})
        if not isinstance(locales, dict) or set(locales) - LOCALES:
            raise ValueError("Invalid theme description locales")
        if locales:
            entry["description_locales"] = {
                locale: text(value, "localized description", multiline=True)
                for locale, value in sorted(locales.items())
            }
    return entry


def catalog(root=ROOT):
    packages = [package(path.parent) for path in sorted(root.glob("*/theme.json"))]
    if not packages:
        raise ValueError("No theme packages found")
    return {"version": 1, "themes": packages}


def generate(root=ROOT, check=False):
    encoded = json.dumps(catalog(root), ensure_ascii=False, indent=2) + "\n"
    target = root / "catalog.json"
    if target.is_symlink():
        raise ValueError("Theme catalog cannot use symbolic links")
    if check:
        if not target.is_file() or target.read_text(encoding="utf-8") != encoded:
            raise ValueError("Catalog is stale; run python3 scripts/catalog.py")
    else:
        target.write_text(encoded, encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="reject a stale index without writing")
    args = parser.parse_args()
    try:
        generate(check=args.check)
    except (OSError, ValueError, KeyError, TypeError) as error:
        raise SystemExit(str(error)) from error


if __name__ == "__main__":
    main()
