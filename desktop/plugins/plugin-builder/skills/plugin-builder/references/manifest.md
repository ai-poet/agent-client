# Package and manifest

## Layout

```text
<name>/
  plugin.json                     required manifest
  skills/<skill>/SKILL.md         optional skills, one folder each
  mcp.json                        optional MCP servers
  dev.sailry.platform/            every script, locale module and asset Sailry loads
    settings.json                 optional settings form schema
    host/main.js                  optional headless callbacks
    desktop/main.js               optional desktop entry
```

Package names are lowercase kebab-case (`task-notes`) and must equal the folder name. Versions are semantic versions such as `0.1.0`.

## plugin.json

```json
{
  "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
  "name": "release-notes",
  "version": "0.1.0",
  "description": "Draft release notes from recent changes",
  "extensions": {
    "dev.sailry.platform": {
      "api_version": "v1",
      "actions": [],
      "display": {
        "label": "Release notes",
        "locales": {"zh-CN": "发布说明"},
        "icon": "phosphor:file-text"
      },
      "description": {
        "label": "Draft release notes from recent changes",
        "locales": {"zh-CN": "根据最近的变更起草发布说明"}
      }
    }
  }
}
```

Unknown fields are rejected anywhere in the extension. Only include the sections below that the package uses.

| Field | Purpose |
| --- | --- |
| `api_version` | Extension API supported by the target application, currently `"v1"` |
| `min_sailry_version` | Optional exact SemVer floor, such as `"0.1.0-alpha.4"`; omission or `null` imposes no minimum |
| `scope` | `"host"` (default): runs on the execution Node and works from every controller. `"desktop"`: bound to the desktop's local Node |
| `actions` | Capabilities the package needs; see below |
| `enabled_service` | Optional `"code_index"` binding for a Host package with `code.index.read`; follows the package's enabled choice |
| `display` | Localized catalog name and icon |
| `description` | Localized one-line description |
| `icon` | Package-relative PNG for catalogs, as an alternative to a `reicon:` icon in `display` |
| `instructions` | Short package instructions supplied to the model together with the package's tools |
| `conversation_options` | Public per-conversation choices stored in this package's options namespace |
| `conversation_modes` | Host mode declarations for the shared selector, with instructions and write restrictions |
| `settings_schema` | Path to a settings form schema, such as `dev.sailry.platform/settings.json` |
| `settings_page` | Settings navigation entry, with an optional custom desktop `entry` |
| `registration` | Provider application creation bound to public and protected settings fields |
| `storage` | Optional JSON schemas for private storage collections |
| `tools` | Agent tools; see `tools.md` |
| `host` | Headless callback entry; see `host.md` |
| `desktop`, `ui` | Panels, navigation, assistants and controls; see `desktop.md` |
| `cards` | Controller-rendered content cards owned by the package |

### Localized text

Every label uses the same shape. `label` is the default (English) text; `locales` adds translations keyed by locale, for example `"zh-CN"`.

```json
{"label": "Release notes", "locales": {"zh-CN": "发布说明"}}
```

### Icons

The package's own `display.icon`, and a connector's, is drawn on a tile in catalogs: name a [Phosphor](https://phosphoricons.com) icon as `phosphor:<name>`, for example `phosphor:file-text`, `phosphor:database` or `phosphor:calendar-check` (the fill weight is implied and no weight suffix is written), or, for a package or connector of one brand, that brand's mark as `ailogo:<slug>`, for example `ailogo:github`; another company's mark is never a substitute. A package that names a Phosphor icon sets `min_sailry_version` to `0.1.6`.

Every other icon is drawn in the text color beside the application's outline icons: navigation, panels, settings pages, tools, menus and view `Icon` components. Use the outline set there as `reicon:<group>/<name>`, for example `reicon:files/file-text`, `reicon:ui/database`, `reicon:time/calendar` or `reicon:notifications/bell`. A `phosphor:` name in such a place still draws, but solid; navigation and panel icons do not accept `ailogo:`. Composer controls in `ui` use plain control icon names such as `eye`, `file-text` or `rotate-cw`. Reuse names that existing official plugins use when unsure.

## Conversation modes

Host packages can extend the shared Build/Plan selector without a custom composer
control. Bind each mode to a string conversation option with an empty default and
an enum containing `""` and the IDs of modes using that field:

```json
"conversation_options": {
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "properties": {"mode": {"type": "string", "default": "", "enum": ["", "review"]}}
},
"conversation_modes": [{
  "id": "review",
  "option": "mode",
  "display": {"label": "Review"},
  "permissions": {"write": false},
  "instructions": "Inspect the requested changes and report findings"
}]
```

Declare up to 16 distinct modes. Node validates the declarations during
installation. `permissions.write: false` requires Plan; `true` uses Build and
still requires the session's ordinary tool permissions. Instructions apply to the
main coordinator; children retain their task, role and captured restrictions.
Exactly one package mode can be selected. Mode changes use the existing session
revision, and selecting Build or Plan clears active mode choices while preserving
other options. Package instructions and options freeze at turn admission; package
updates or removal never rewrite those captures.

## Actions

Declare only what the package uses. Each action grants one capability family:

| Action | Grants |
| --- | --- |
| `storage.read`, `storage.write` | The package's private key-value storage on the Node |
| `http.request` | HTTP requests made by the Node |
| `files.read`, `files.write` | Files in the captured worktree |
| `code.index.read` | Indexed source symbols and file structure |
| `code.index.settings.read`, `code.index.settings.write` | Read or change the Node code-index setting |
| `git.read`, `git.write` | Git status and history; staging and commits |
| `worktrees.read`, `worktrees.write` | Registered worktrees |
| `projects.read` | The registered project catalog |
| `conversation.read`, `conversation.control` | The captured conversation; sending messages and stopping turns |
| `conversation.message.read` | The current turn's user text in the `turn.initialize` input |
| `sessions.start` | Starting new conversations |
| `models.read`, `models.install`, `models.generate` | The model catalog, managed installation and model requests through the Node |
| `decisions.run` | Local model decisions |
| `notifications.publish` | Application notifications |
| `dispatch.manage` | Scheduled and queued plugin work |
| `process.read`, `process.control`, `terminals.read`, `terminals.control` | Commands and terminals |
| `process.run` | Start the installed executables the package declares in `programs` |
| `databases.read`, `databases.control`, `databases.manage` | Database connections |
| `ssh.read`, `ssh.control`, `ssh.manage` | SSH connections |
| `browser.read`, `browser.control`, `external_browser.read`, `external_browser.control` | Built-in and external browsers |
| `computer.read`, `computer.control` | Screen capture and desktop control |
| `media.settings.read`, `media.settings.write` | The media model assignments. Image inspection and image and video generation are built-in agent tools; `media.inspect`, `media.image` and `media.video` still validate but bind no tool |
| `agents.delegate`, `roles.read`, `roles.write` | Subagents and delegation roles |
| `teams.read` | The team catalog |
| `activity.read`, `usage.read` | Activity and usage reports |

A skill-only package needs `"actions": []`.

## Skills

Each skill is `skills/<skill>/SKILL.md` with YAML frontmatter. The skill `name` must equal its folder name; it uses lowercase letters, digits and single hyphens, up to 64 characters. `description` is required (up to 1024 characters) and tells the model when to load the skill. `metadata` values must be strings.

```markdown
---
name: release-notes
metadata:
  display-name: Release notes
description: Draft release notes from recent commits and merged changes. Use when the user asks for a changelog or release summary.
---

# Release notes

1. Read the Git log since the last tag with the available Git tools.
2. Group changes into Added, Changed and Fixed.
3. Write `RELEASE_NOTES.md` only when the user asks for a file.
```

Skill files are limited to 128 KiB, with up to 64 skills per package. Other files in the skill folder are resources the model reads with `read_skill_resource`; scripts run through the ordinary command tool, resolved against the directory returned by `load_skill`.

## Settings form

`settings_schema` points to a bounded JSON Schema that Sailry renders as a native form. Up to 32 fields, each `string`, `integer`, `number` or `boolean`.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "base_url": {"type": "string", "title": "Server", "default": "https://api.example.com"},
    "token": {
      "type": "string",
      "title": "Token",
      "x-sailry-secret": {"origin": "https://api.example.com", "header": "Authorization", "prefix": "Bearer "}
    },
    "model": {"type": "string", "title": "Model", "x-sailry-model": true}
  },
  "required": ["base_url"],
  "x-sailry-locales": {"zh-CN": {"base_url": "服务器", "token": "令牌", "model": "模型"}}
}
```

- `x-sailry-secret` stores the value in the Node's protected storage. Code never reads it; the Node injects it only into HTTP requests for that origin when a request names the field as its `credential`.
- `x-sailry-model: true` renders a model picker. A separate string field with `"x-sailry-model-effort": "model"` renders the reasoning-effort picker for the model field named `model`.
- `x-sailry-tabs` groups fields into tabs: `[{"id": "general", "title": "General", "fields": ["base_url"]}]`.
- Add `"settings_page": {"navigation": {"label": "Release notes"}}` to give the form its own entry in Settings.

An optional `registration` binds provider-owned application creation to existing
settings fields. Its `client_id` names a public string field and `client_secret`
names that provider's protected application-secret field. Keep provider scopes in
`tenant_scopes`; registration and account OAuth are separate declarations.

Read values with `readSettings()` from the SDK, or with a tool bound to the `settings.read` operation.
