"""Preflight check for a Sailry plugin package folder.

Usage: python3 check_package.py <package-folder>

Reports likely problems before installation. Sailry validates the package again
when it is installed; that result is authoritative.
"""

import json
import re
import sys
from pathlib import Path

EXTENSION = "dev.sailry.platform"
PREFIX = EXTENSION + "/"
NAME = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
NUMBER = r"(?:0|[1-9]\d*)"
PART = r"(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)"
VERSION = re.compile(
    rf"^{NUMBER}\.{NUMBER}\.{NUMBER}(?:-{PART}(?:\.{PART})*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$")
IMPORT = re.compile(
    r"""(?:\b(?:import|export)\b[^;'"`]*?\bfrom\s*|\bimport\s*\(?\s*)['"]([^'"]+)['"]""")
MODULES = {
    "gpui-base", "gpui-component", "gpui-kit", "sailry", "sailry/ui", "sailry/forms",
    "sailry/sdk", "sailry/connections", "sailry/credentials", "sailry/models",
    "sailry/card", "sailry/cards", "sailry/connectors", "sailry/http",
    "sailry/registration", "sailry/documents",
    "sailry/file-transfers", "sailry/ssh-transfers", "sailry/artifacts", "sailry/search",
    "sailry/settings", "sailry/installations",
}
FIELDS = {
    "api_version", "min_sailry_version", "scope", "actions", "display", "description", "icon", "instructions",
    "settings_schema", "settings_page", "storage", "tools", "host",
    "desktop", "ui", "artifact_view", "continuation_marker", "continuation_markers", "credential_source",
    "enabled_service", "oauth", "registration", "permissions", "connectors", "cards", "category", "tint",
    "conversation_options", "conversation_modes", "model_providers", "model_auth", "model_resources", "model_adapters", "programs",
}
ACTIONS = {
    "activity.read", "usage.read", "databases.read", "databases.control", "databases.manage",
    "ssh.read", "ssh.control", "ssh.manage", "agents.delegate", "roles.read", "teams.read", "roles.write",
    "models.read", "decisions.run", "models.install", "models.provide", "models.authenticate", "credentials.inspect",
    "media.inspect", "media.image", "media.video", "media.settings.read",
    "media.settings.write", "computer.read", "computer.control", "browser.read",
    "browser.control", "external_browser.read", "external_browser.control",
    "notifications.publish", "dispatch.manage", "files.read", "files.write", "git.read",
    "git.write", "worktrees.read", "worktrees.write", "process.read", "process.control", "process.run",
    "terminals.read", "terminals.control", "conversation.read", "conversation.control",
    "conversation.message.read",
    "sessions.start", "projects.read", "http.request", "models.generate", "storage.read",
    "storage.write", "code.index.read", "artifacts.read", "artifacts.write",
    "code.index.settings.read", "code.index.settings.write", "web.read", "web.settings.read",
    "web.settings.write", "knowledge.read", "knowledge.write",
}
OPERATION_ACTIONS = {
    "models.install": "models.install", "models.remove": "models.install",
    "code.search": "code.index.read", "code.outline": "code.index.read",
    "storage.get": "storage.read", "storage.list": "storage.read",
    "storage.set": "storage.write", "storage.delete": "storage.write",
    "http.request": "http.request", "files.list": "files.read", "files.read": "files.read",
    "files.read_lines": "files.read",
    "files.search": "files.read", "files.find": "files.read", "files.write": "files.write",
    "files.edit": "files.write", "git.status": "git.read", "git.diff": "git.read",
    "git.log": "git.read", "git.show": "git.read", "git.blame": "git.read",
    "git.changes": "git.read", "git.change": "git.read",
    "worktrees.list": "worktrees.read",
}
RETIRED_OPERATIONS = {
    "code.packages", "code.catalog", "code.servers", "code.start", "code.stop",
    "code.install", "code.uninstall", "code.diagnostics", "code.definition",
    "code.references", "code.hover", "code.symbols",
}
STORAGE_OPERATIONS = {
    "storage.get", "storage.list", "storage.set", "storage.delete", "storage.transaction",
}
SETTING_KINDS = {"string", "integer", "number", "boolean"}


class Report:
    def __init__(self):
        self.errors = []
        self.warnings = []

    def error(self, message):
        self.errors.append(message)

    def warn(self, message):
        self.warnings.append(message)


def load_json(path, report):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        report.error(f"{path.name} is missing")
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        report.error(f"{path.name} is not valid JSON: {error}")
    return None


def package_file(root, value, label, report):
    """Resolve a declared resource path, or report why it is invalid."""
    if not isinstance(value, str) or not value:
        report.error(f"{label} must be a nonempty path")
        return None
    path = value[2:] if value.startswith("./") else value
    parts = path.split("/")
    if "\\" in path or ":" in path or any(part in ("", ".", "..") for part in parts):
        report.error(f"{label} has an invalid path: {value}")
        return None
    if not path.startswith(PREFIX):
        report.error(f"{label} must be under {PREFIX}: {value}")
        return None
    target = (root / path).resolve()
    if not target.is_relative_to(root.resolve()):
        report.error(f"{label} escapes the package: {value}")
        return None
    if not target.is_file():
        report.error(f"{label} does not exist: {value}")
        return None
    return target


def check_names(values, label, report):
    if not isinstance(values, list) or any(not isinstance(v, str) or not v for v in values):
        report.error(f"{label} must be a list of nonempty strings")
        return []
    if len(set(values)) != len(values):
        report.error(f"{label} contains duplicates")
    return values


def check_sections(root, extension, report):
    """Check entries and resources, then the imports of every script."""
    declared = {}
    sections = [(key, extension.get(key), False) for key in ("host", "desktop", "settings_page")]
    sections.extend(("model_adapters.script", item.get("script"), True)
                    for item in extension.get("model_adapters", []) if isinstance(item, dict) and "acp" not in item)
    for item in extension.get("model_auth", []):
        if not isinstance(item, dict):
            continue
        if isinstance(item.get("exchange"), dict):
            sections.append(("model_auth.exchange.script", item["exchange"].get("script"), True))
        if isinstance(item.get("flow"), dict) and item["flow"].get("kind") == "script":
            sections.append(("model_auth.flow.script", item["flow"].get("script"), True))
    pure = set()
    for key, section, translation in sections:
        if not isinstance(section, dict):
            continue
        resources = check_names(section.get("resources", []), f"{key}.resources", report)
        if translation and isinstance(section.get("entry"), str) and section["entry"] not in resources:
            resources = [*resources, section["entry"]]
        for value in resources:
            target = package_file(root, value, f"{key}.resources", report)
            if target:
                declared[target] = value
                if translation:
                    pure.add(target)
        for field in ("entry", "ui_entry"):
            value = section.get(field)
            if value is None:
                continue
            target = package_file(root, value, f"{key}.{field}", report)
            if target and target not in declared:
                report.error(f"{key}.{field} must also be listed in {key}.resources: {value}")
    for target, value in declared.items():
        if target.suffix not in (".js", ".mjs"):
            continue
        source = target.read_text(encoding="utf-8", errors="replace")
        for specifier in IMPORT.findall(source):
            if not specifier.startswith("."):
                if (specifier != "sailry/adapter" if target in pure else specifier not in MODULES):
                    report.error(f"{value} imports an unavailable module: {specifier}")
                continue
            imported = (target.parent / specifier).resolve()
            if imported not in declared:
                report.error(f"{value} imports an undeclared resource: {specifier}")


def check_tools(extension, servers, report):
    actions = set(extension.get("actions") or [])
    handlers = set((extension.get("host") or {}).get("handlers") or [])
    tools = extension.get("tools", [])
    if not isinstance(tools, list):
        report.error("tools must be a list")
        return
    check_names([tool.get("name") for tool in tools if isinstance(tool, dict)], "tool names", report)
    for tool in tools:
        if not isinstance(tool, dict) or not isinstance(tool.get("name"), str):
            report.error("each tool needs a name")
            continue
        name = tool["name"]
        kinds = [key for key in ("operation", "handler", "server") if tool.get(key) is not None]
        if len(kinds) != 1:
            report.error(f"tool {name} needs exactly one of operation, handler or server")
            continue
        handler = tool.get("handler")
        if handler is not None and not isinstance(handler, dict):
            report.error(f"tool {name} handler must be an object")
            continue
        if isinstance(handler, dict):
            if handler.get("name") not in handlers:
                report.error(f"tool {name} handler {handler.get('name')!r} is not in host.handlers")
            if "parameters" not in handler:
                report.error(f"tool {name} handler needs a parameters schema")
        operation = tool.get("operation") or (handler or {}).get("operation")
        operations = [operation] if operation is not None else []
        flow = (handler or {}).get("flow")
        declared = operation is not None or flow is not None or (handler or {}).get("read_only") is True
        if flow is not None:
            if not isinstance(flow, dict):
                report.error(f"tool {name} flow must be an object")
            else:
                operations += check_names(flow.get("operations", []),
                                          f"tool {name} flow.operations", report)
        for operation in operations:
            if not isinstance(operation, str):
                report.error(f"tool {name} operation must be a string")
                continue
            if operation in RETIRED_OPERATIONS:
                report.error(f"tool {name} uses a retired operation: {operation}")
                continue
            needed = OPERATION_ACTIONS.get(operation)
            if needed and needed not in actions:
                report.error(f"tool {name} uses {operation} and needs the {needed} action")
        initial = tool.get("initial", False)
        if not isinstance(initial, bool):
            report.error(f"tool {name} initial must be a boolean")
        elif initial:
            # Initially visible tools may only read or change the package's own storage.
            if tool.get("server") is not None or not declared or any(
                    item not in STORAGE_OPERATIONS for item in operations):
                report.error(f"tool {name} initial needs a read_only handler or only storage operations")
        server = tool.get("server")
        if server is not None and server not in servers:
            report.error(f"tool {name} names server {server!r}, which mcp.json does not declare")


def check_host(extension, report):
    host = extension.get("host")
    if host is None:
        return
    if not isinstance(host, dict):
        report.error("host must be an object")
        return
    for field in ("entry", "handlers", "resources"):
        if field not in host:
            report.error(f"host.{field} is required")
    check_names(host.get("handlers", []), "host.handlers", report)
    hooks = list((host.get("turn") or {}).values())
    hooks += [command.get("handler") for command in host.get("commands", []) if isinstance(command, dict)]
    for hook in filter(None, hooks):
        if hook not in host.get("handlers", []):
            report.error(f"host handler {hook!r} must be listed in host.handlers")


def check_desktop(extension, report):
    desktop = extension.get("desktop")
    if not isinstance(desktop, dict):
        return
    navigation = desktop.get("navigation")
    if isinstance(navigation, dict) and not navigation.get("icon"):
        report.error("desktop.navigation needs an icon")
    has_controls = any(isinstance(item, dict) and item.get("slot") != "statistics" for item in extension.get("ui", []))
    if has_controls and not desktop.get("ui_entry"):
        report.warn("ui controls need desktop.ui_entry to handle their events")


def check_settings(root, extension, report):
    value = extension.get("settings_schema")
    if value is None:
        return
    target = package_file(root, value, "settings_schema", report)
    schema = load_json(target, report) if target else None
    if not isinstance(schema, dict):
        return
    if schema.get("type") != "object" or not isinstance(schema.get("additionalProperties"), bool):
        report.error("settings schema needs \"type\": \"object\" and a boolean additionalProperties")
    if "$schema" not in schema:
        report.error("settings schema needs $schema")
    fields = schema.get("properties", {})
    if not isinstance(fields, dict) or len(fields) > 32:
        report.error("settings schema allows up to 32 properties")
        return
    for name, field in fields.items():
        if not isinstance(field, dict) or field.get("type") not in SETTING_KINDS:
            report.error(f"setting {name} must be a string, integer, number or boolean")


def frontmatter(text):
    """Return top-level scalar keys from simple YAML frontmatter."""
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        return None
    values = {}
    for line in lines[1:]:
        if line.strip() == "---":
            return values
        match = re.match(r"^([A-Za-z][\w-]*):\s*(.*)$", line)
        if match:
            values[match.group(1)] = match.group(2).strip().strip("'\"")
    return None


def check_skills(root, report):
    directory = root / "skills"
    if not directory.is_dir():
        return
    folders = sorted(path for path in directory.iterdir() if path.is_dir())
    if len(folders) > 64:
        report.error("a package allows up to 64 skills")
    for folder in folders:
        skill = folder / "SKILL.md"
        if not skill.is_file():
            report.error(f"skills/{folder.name} has no SKILL.md")
            continue
        if skill.stat().st_size > 128 * 1024:
            report.error(f"skills/{folder.name}/SKILL.md is larger than 128 KiB")
        header = frontmatter(skill.read_text(encoding="utf-8", errors="replace"))
        if header is None:
            report.error(f"skills/{folder.name}/SKILL.md needs YAML frontmatter between --- lines")
            continue
        if header.get("name") != folder.name or not NAME.match(folder.name) or len(folder.name) > 64:
            report.error(f"skills/{folder.name}: name must equal the folder name in lowercase kebab-case")
        description = header.get("description", "")
        if not description or len(description) > 1024:
            report.error(f"skills/{folder.name}: description is required, up to 1024 characters")


def check_servers(root, report):
    path = root / "mcp.json"
    if not path.exists():
        return set()
    data = load_json(path, report)
    servers = data.get("mcpServers") if isinstance(data, dict) else None
    if not isinstance(servers, dict) or not servers:
        report.error("mcp.json needs a nonempty mcpServers object")
        return set()
    for name, server in servers.items():
        if not isinstance(server, dict) or not (server.get("command") or server.get("url")):
            report.error(f"MCP server {name} needs a command or a url")
    return set(servers)


def check(root):
    report = Report()
    manifest = load_json(root / "plugin.json", report)
    if not isinstance(manifest, dict):
        return report
    name = manifest.get("name")
    if not isinstance(name, str) or not NAME.match(name):
        report.error("name must be lowercase kebab-case, for example task-notes")
    elif name != root.name:
        report.error(f"name {name!r} must equal the folder name {root.name!r}")
    if not isinstance(manifest.get("version"), str) or not VERSION.match(manifest["version"]):
        report.error("version must be a semantic version, for example 0.1.0")
    servers = check_servers(root, report)
    check_skills(root, report)
    extension = (manifest.get("extensions") or {}).get(EXTENSION)
    if extension is None:
        report.warn(f"no {EXTENSION} extension; Sailry loads it as a plain Agent Plugins package")
        return report
    if not isinstance(extension, dict):
        report.error(f"{EXTENSION} must be an object")
        return report
    if extension.get("api_version") != "v1":
        report.error('api_version must be "v1"')
    minimum = extension.get("min_sailry_version")
    if minimum is not None and (
        not isinstance(minimum, str) or not minimum.isascii() or not VERSION.fullmatch(minimum)
    ):
        report.error("min_sailry_version must be a semantic version, for example 0.1.0-alpha.4")
    for field in sorted(set(extension) - FIELDS):
        report.error(f"unknown extension field: {field}")
    for field, choices in (
        ("category", {"development", "documents", "data", "automation", "entertainment"}),
        ("tint", {"blue", "violet", "teal", "amber", "gray"}),
    ):
        value = extension.get(field)
        if value is not None and (not isinstance(value, str) or value not in choices):
            report.error(f"{field} must be one of: {', '.join(sorted(choices))}")
    actions = check_names(extension.get("actions", []), "actions", report)
    for action in actions:
        if action not in ACTIONS:
            report.error(f"unknown action: {action}")
    service = extension.get("enabled_service")
    if service is not None:
        if service != "code_index":
            report.error("enabled_service must be code_index")
        else:
            if extension.get("scope", "host") != "host":
                report.error("code_index requires host scope")
            if "code.index.read" not in actions:
                report.error("code_index needs the code.index.read action")
    if not isinstance(extension.get("display"), dict):
        report.warn("display is missing; catalogs fall back to the package name")
    if not manifest.get("description") and not extension.get("description"):
        report.warn("add a description so the catalog can explain the plugin")
    for auth in extension.get("model_auth", []):
        if not isinstance(auth, dict):
            continue
        scripted = isinstance(auth.get("flow"), dict) and auth["flow"].get("kind") == "script"
        if (scripted or auth.get("exchange")) and "credentials.inspect" not in extension.get("actions", []):
            report.error("credential transformations require credentials.inspect")
    check_sections(root, extension, report)
    check_host(extension, report)
    check_tools(extension, servers, report)
    check_desktop(extension, report)
    check_settings(root, extension, report)
    return report


def main(arguments):
    if len(arguments) != 1:
        print("usage: check_package.py <package-folder>", file=sys.stderr)
        return 2
    root = Path(arguments[0]).expanduser()
    if not root.is_dir():
        print(f"error: {root} is not a folder", file=sys.stderr)
        return 2
    report = check(root.resolve())
    for message in report.errors:
        print(f"error: {message}")
    for message in report.warnings:
        print(f"warning: {message}")
    if report.errors:
        print(f"{len(report.errors)} error(s); fix them before installing")
        return 1
    print("ok: ready to install from Settings → Plugins")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
