# Desktop surfaces

Desktop code runs in moejs and renders native MyGo and Kit components through the `gpui-kit` bindings. It runs only in Sailry Desktop; Mobile and Host use the package's skills and tools. Desktop code is trusted application code, not an operating-system sandbox, so keep it small and request only the actions it needs.

## Entry points

```json
"desktop": {
  "entry": "dev.sailry.platform/desktop/main.js",
  "resources": ["dev.sailry.platform/desktop/main.js", "dev.sailry.platform/desktop/locales.js"],
  "panel": {"label": "Release notes", "locales": {"zh-CN": "发布说明"}},
  "navigation": {"label": "Release notes", "locales": {"zh-CN": "发布说明"}, "icon": "reicon:files/file-text"}
}
```

| Field | Surface |
| --- | --- |
| `panel` | An entry in the conversation's resource panel, bound to that conversation's Node and worktree |
| `navigation` | A page in the application sidebar; requires `icon`. `navigation_options.target` is `"node"` or `"worktree"` |
| `conversations` | Named assistants that can be embedded with the `Conversation` component |
| `ui_entry` | A controller module for `ui` contributions; it renders no layout of its own |

`resources` lists every module the entry imports, and every path stays under `dev.sailry.platform/`.

## Allowed modules

| Module | Common exports |
| --- | --- |
| `gpui-kit` | `View`, `div` |
| `gpui-component` | `Button`, `Checkbox`, `Toggle`, `Tag`, `Spinner`, `Icon`, `TabBar`, `Tab`, `Accordion`, `VForm`, `Field` |
| `sailry` | `context`, `theme`, `Header`, `header_action`, `Image`, `Conversation`, `next_change` |
| `sailry/ui` | `toast`, `Tooltip`, `Modal`, `EmptyState`, `IconButton`, `SelectField`, `CardList`, `CardRow`, `SettingsPage`, `Loading` |
| `sailry/sdk` | Settings, storage, models, HTTP, Git, files and conversation requests |
| `sailry/card`, `sailry/cards` | Package-owned content cards and host-side card blocks |
| `sailry/registration` | Application registration controls and observation |

Other host modules: `gpui-base`, `sailry/forms`, `sailry/connections`, `sailry/credentials`, `sailry/models`, `sailry/documents`, `sailry/file-transfers`, `sailry/ssh-transfers`. Any other bare import is rejected, and npm packages are not available. Prefer the components above over hand-drawn controls, and take colors from `theme().colors` instead of hard-coded values.

## A minimal panel

```js
import {View, div} from "gpui-kit";
import {Button} from "gpui-component";
import {context, theme} from "sailry";
import {toast} from "sailry/ui";
import {getValue, setValue, completeRequest, forgetRequest} from "sailry/sdk";
import {messages} from "./locales.js";

// Faults that leave the outcome unknown; keep the request ID and retry it later.
const uncertain = ["outcome_unknown", "unavailable", "busy", "internal", "cancelled"];

export default class Counter extends View {
  init(_props, cx) {
    this.text = messages(JSON.parse(context()).locale);
    this.entry = null;
    this.pending = null;
    this.busy = false;
    cx.spawn(async (cx) => {
      this.entry = await getValue("count");
      cx.notify();
    });
  }

  increment(cx) {
    if (!this.entry || this.busy) return;
    // An earlier uncertain write is retried with its original ID, never prepared again.
    if (!this.pending) this.pending = setValue("count", (this.entry.value ?? 0) + 1, this.entry.revision);
    this.busy = true;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const result = await completeRequest(this.pending);
        if (uncertain.includes(result.Err?.code)) throw new Error(result.Err.code);
        if (result.Err) toast({id: "counter", kind: "error", message: this.text.failed});
        forgetRequest(this.pending);
        this.pending = null;
        this.entry = await getValue("count");
      } catch (_) {
        // Keep this.pending: the outcome is uncertain, and a retry must reuse the same ID.
        toast({id: "counter", kind: "error", message: this.text.unknown});
      }
      this.busy = false;
      cx.notify();
    });
  }

  render(cx) {
    return div().v_flex().size_full().p_4().gap_3()
      .child(div().text_sm().text_color(theme().colors.muted_foreground)
        .child(`${this.text.count}: ${this.entry?.value ?? 0}`))
      .child(new Button("counter-add").primary().label(this.text.add)
        .disabled(!this.entry || this.busy)
        .on_click((_event, cx) => this.increment(cx)));
  }
}
```

```js
// locales.js
const strings = {
  en: {count: "Count", add: "Add one", failed: "Couldn't save", unknown: "Save may not have finished"},
  "zh-CN": {count: "计数", add: "加一", failed: "保存失败", unknown: "保存结果未知"},
};
export const messages = (locale) => strings[locale] ?? strings.en;
```

This package needs `"actions": ["storage.read", "storage.write"]`.

- Build layout with `div()` chains (`v_flex`, `h_flex`, `gap_*`, `p_*`, `size_full`, `flex_1`, `min_h_0`, `overflow_y_scroll`, `text_sm`). Give interactive elements stable string IDs.
- Do slow work in `cx.spawn(async (cx) => …)` and call `cx.notify()` after changing state.
- Report operation results with `toast`, not text left in the panel.
- `context()` from `sailry` returns JSON with the `locale` and the captured scope. Closing a panel releases it without cancelling work the Node already admitted.

## Embedded assistants

```json
"actions": ["conversation.read", "conversation.control"],
"desktop": {
  "entry": "dev.sailry.platform/desktop/chat.js",
  "resources": ["dev.sailry.platform/desktop/chat.js"],
  "panel": {"label": "Notes"},
  "conversations": [{
    "id": "notes",
    "resource": "plugin",
    "context": "Help the user organize their notes",
    "tools": [{"kind": "plugin", "name": "read_note"}, {"kind": "plugin", "name": "save_note"}]
  }]
}
```

```js
import {View, div} from "gpui-kit";
import {Conversation, context} from "sailry";

export default class Chat extends View {
  render() {
    const props = JSON.parse(context()).session ? undefined : {assistant: "notes"};
    return div().size_full().child(Conversation.new("notes-chat", props));
  }
}
```

`resource` is `plugin` (a private workspace), `workspace` (the panel's project), `database` or `ssh`. Tool selectors use `{"kind": "plugin", "name": …}` for this package's tools, `{"kind": "builtin", "name": …}` for native tools, and `{"kind": "package", "package": …, "name": …}` for another package's tool.

## Composer controls

Declare `conversation_modes` for shared mode selection; no `ui_entry` is needed.
`readComposer()` exposes the available `modes`, current `mode_selection: {package,
id} | null` and effective `mode: 'code' | 'plan'`. With `conversation.control`,
select this package's declaration through
`setComposerOption(cursor, 'mode', {package, id})`, or select a builtin with
`'code'` or `'plan'`. The host applies revision checks and clears other active
mode choices while preserving unrelated options.

`ui` entries add native controls to the conversation without drawing them yourself. Slots: `composer`, `context`, `statistics`, `status`, `project`, `project_menu`, `commands`. Kinds: `button`, `toggle`, `select`, `menu`, `picker`, `popover`, `metric`, `indicator`.

```json
"ui": [{
  "id": "focus",
  "slot": "composer",
  "kind": "toggle",
  "label": {"label": "Focus", "locales": {"zh-CN": "专注"}},
  "icon": "eye",
  "order": 50,
  "handler": "focus"
}],
"desktop": {"ui_entry": "dev.sailry.platform/desktop/controls.js", "resources": ["dev.sailry.platform/desktop/controls.js"]}
```

The `ui_entry` module publishes state and handles events:

```js
import {View, div} from "gpui-kit";
import {publishContributions, nextContributionEvent} from "sailry/sdk";

export default class Controls extends View {
  init(_props, cx) {
    this.focus = false;
    cx.spawn(async () => {
      publishContributions([{id: "focus", value: this.focus}]);
      while (true) {
        const event = await nextContributionEvent();
        if (event.handler === "focus") this.focus = event.value;
        publishContributions([{id: "focus", value: this.focus, reply_to: event.sequence}]);
      }
    });
  }
  render() { return div(); }
}
```

Reply to each event with `reply_to: event.sequence`; a control stays disabled until its reply arrives. Add `"overflow": "menu"` to move a control into the composer's settings menu. The Task notes example in the official plugins repository shows every kind.
