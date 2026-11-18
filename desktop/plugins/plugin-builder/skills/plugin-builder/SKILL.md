---
name: plugin-builder
metadata:
  display-name.en: "Plugin builder"
  display-name.zh-CN: "插件开发"
  display-name.zh-TW: "外掛程式開發"
  display-name.ja: "プラグイン開発"
  display-name.ko: "플러그인 개발"
  display-name.fr: "Création de plugins"
  display-name.de: "Plugin-Entwicklung"
  display-name.es: "Creación de plugins"
  display-name.pt-BR: "Criação de plugins"
  display-name.ru: "Разработка плагинов"
  display-name: Plugin builder
description: Create, extend and fix Sailry plugins - skills, agent tools, MCP servers, headless callbacks, desktop panels and composer controls - and prepare them for installation.
---

# Plugin builder

Build a Sailry plugin package in the user's project, check it, and guide installation. A package is a folder with `plugin.json` at its root, using the Agent Plugins manifest and Sailry's `dev.sailry.platform` v1 extension.

## Choose the smallest package

Start from what the user wants the plugin to do. Pick the first level that is enough, and combine levels only when needed:

| Need | Package contents | Code | Reference |
| --- | --- | --- | --- |
| Reusable instructions or a workflow | `skills/<name>/SKILL.md` | None | `references/manifest.md` |
| Tools for storage, HTTP, progress or settings | `tools` bound to a host `operation` | None | `references/tools.md` |
| Tools with custom logic | `tools` with a `handler` and a `host` entry | JavaScript (moejs) | `references/tools.md`, `references/host.md` |
| An existing command-line or MCP server | `mcp.json` and `tools` with `server` | Any language | `references/tools.md` |
| A panel, sidebar page, assistant or composer control | `desktop` and `ui` sections | JavaScript (MyGo/Kit bindings) | `references/desktop.md` |

Ask only for what you cannot infer: the plugin's purpose, and where the package folder should live. Default to `plugins/<name>/` inside the current worktree. Do not write into Sailry's application or profile directories.

## Workflow

1. Read `references/manifest.md` with `read_skill_resource`, then the reference for each level you chose.
2. Write the package files in the worktree. Use a kebab-case `name` that matches the folder name and a semantic `version`, starting at `0.1.0`.
3. Keep every script, locale module and asset that a section loads under `dev.sailry.platform/`, and list it in that section's `resources`.
4. Put user-visible text in `label` plus `locales` (for example `"zh-CN"`) instead of hard-coding one language. Keep labels short.
5. Run the preflight check and fix every error it reports:

   ```sh
   "<python>" "<skill-directory>/scripts/check_package.py" "<package-folder>"
   ```

   Use the absolute `python` and `directory` returned by `load_skill`. If managed Python or command permission is unavailable, report that the check did not run.
6. Tell the user how to install or update it, using the steps below.

When changing an existing plugin, read its `plugin.json` and the files it loads first, and preserve unrelated declarations. Increment its semantic version for changed content; ordinary updates increment the patch version, such as `0.1.0` to `0.1.1`. Update detection requires a strictly newer version; same-version content changes and build metadata alone do not offer an update. Use the extension API supported by the target Sailry application, currently `v1`. Content digests still verify immutable resources; installed configuration and historical captures remain preserved.

## Install and update

Installation is the user's decision, because desktop plugin code runs with the trust of the application and is not sandboxed by the operating system. Give the user these steps instead of claiming the plugin is installed:

1. Open **Settings → Plugins**, choose **Install plugin**, then **Choose package or directory**, and select the package folder (or a ZIP with `plugin.json` at its root).
2. After later edits, open the plugin in **Settings → Plugins** and choose **Update plugin**. Sailry reloads it from the same folder.
3. If the plugin shows **Unloaded components**, ask the user for those messages and fix the reported declarations.

The folder is chosen on the computer running Sailry Desktop. When the conversation runs on a remote Sailry Host, the package files are on that Host: offer a ZIP the user can download, or ask them to copy the folder to their computer first.

Installed packages are immutable snapshots. Turns that are already running keep the version they started with; new turns use the update.

## Python dependencies

Put a standard `requirements.txt` at the plugin root only when the plugin needs Python libraries. Pin every direct and transitive dependency and include SHA-256 hashes for its wheels. Sailry creates a shared environment keyed by the bundled runtime and exact file contents; explicit installation, enabling, or first skill/MCP use prepares it with hash checks and wheels only. Do not add custom runtime fields, setup scripts, source builds, or automatic parsing of third-party skills.

Write the direct dependencies to an input file. Use the Commands Python skill to create a session-local environment, then install `pip-tools` there under normal command and network permissions. Run that environment's Python on `scripts/lock_requirements.py <input> <package-folder>` from this skill. It generates the complete lock with `pip-compile --generate-hashes`; review pins and platform wheels before publishing. Regenerate the lock when dependencies change and validate installation. Do not hand-assemble hashes, omit transitive dependencies, or install into Sailry's application/shared environment.

## Rules

- Declare only the `actions` the plugin needs. Each action is a capability grant the user reviews; unknown action names grant nothing.
- Use the host modules and SDK functions listed in the references. Do not bundle a copy of `sailry/sdk`, a model client or credentials. Keep secrets in a settings field marked as a secret, never in package files or storage.
- Durable SDK calls return a request ID. Keep that ID until `completeRequest` confirms an outcome, reuse it to recover an uncertain result, and never retry a write under a new ID.
- Keep plugin output honest: report conflicts and uncertain outcomes instead of retrying or claiming success.
- The authoritative contracts live in the Sailry repository: the [extension schema](https://github.com/ai-poet/agent-client/desktop/blob/main/schemas/plugins/v1/extension.schema.json), the [SDK guide](https://github.com/ai-poet/agent-client/desktop/blob/main/sdk/plugins.md), and the [official plugins](https://github.com/sailry/sailry-plugins). Consult them when the references here do not cover a field, and when network access is available.
