# Headless callbacks

The `host` section runs JavaScript on the execution Node, without any desktop view. It powers script tools and callbacks, and works the same from Desktop, Mobile and remote Hosts.

```json
"host": {
  "entry": "dev.sailry.platform/host/main.js",
  "resources": ["dev.sailry.platform/host/main.js", "dev.sailry.platform/shared/format.js"],
  "handlers": ["count", "summarize"]
}
```

- `entry` is an ES module; each name in `handlers` must be a named export.
- `resources` lists the entry and every module it imports. Relative imports must point to declared resources under `dev.sailry.platform/`.
- The only host module is `sailry/sdk`. There is no filesystem, process, `fetch` or Node.js global; use SDK requests instead.
- Callbacks run in a bounded moejs VM. Do not rely on module variables between calls; keep state in storage.

## SDK in callbacks

```js
import {getValue, setValue, completeRequest, forgetRequest} from "sailry/sdk";

export async function remember(args) {
  const entry = await getValue("last");
  const id = setValue("last", args.text, entry.revision);
  const result = await completeRequest(id);
  if (result.Err) throw new Error(result.Err.message);
  forgetRequest(id);
  return {saved: true};
}
```

| Need | Functions | Action |
| --- | --- | --- |
| Settings | `readSettings()` returns `{values, ready}` | None |
| Scope | `context()` | None |
| Node storage | `getValue`, `listKeys`, `setValue`, `deleteValue` | `storage.read` / `storage.write` |
| Conversation storage | `getConversationValue`, `setConversationValue`, `deleteConversationValue` | `storage.read` / `storage.write` |
| Worktree files | `readFile(path)`, `readFileLines(path, offset, limit, expectedRevision?)`, `prepareFile(path, text, expectedRevision)` | `files.read` / `files.write` |
| Code index | `searchCode(query)`, `outlineCode(path)` | `code.index.read` |
| Git | `inspectGit()` | `git.read` |
| Notifications | `prepareNotification({title, message, kind})` | `notifications.publish` |
| Conversation | `readSession()`, `readConversation(before, limit)` | `conversation.read` |
| New conversations | `startSession(draft)` | `sessions.start` |
| Projects | `readProjectCatalog()` | `projects.read` |
| Atomic writes | `prepareTransaction(operations)` (up to 64) | As needed |
| Requests | `prepareRequest(command)`, `completeRequest(id)`, `forgetRequest(id)`, `newId()` | As needed |

Functions named `prepare…`, `set…` and `delete…` only prepare a durable request and return its ID. `completeRequest(id)` submits it and returns `{Ok: {kind, data}}` or `{Err: {code, message}}`. Keep the ID until the outcome is confirmed; on an uncertain outcome, retry with the same ID or report it. Never resubmit a write under a new ID. Revisions are decimal strings; a missing key has revision `"0"`.

Code queries return full-content file revisions and zero-based source ranges with UTF-8 byte columns. Pass the file revision to `readFileLines` before expanding an indexed range. A changed file returns `revision_conflict`; obtain fresh symbols before trying new ranges. The Code plugin owns bounded context composition; callbacks do not install or start language servers.

Model, HTTP and UI functions are desktop-only. A headless tool reaches HTTP through a handler bound to the `http.request` operation (see `tools.md`).

## Calling a callback from a desktop view

```js
import {prepareRequest, completeRequest, forgetRequest} from "sailry/sdk";

const id = prepareRequest({kind: "call_plugin", data: {handler: "summarize", input: {locale: "en"}}});
const result = await completeRequest(id);
if (!result.Err) forgetRequest(id);
```

## Advanced

`host.commands` routes a composer command (a `ui` entry in the `commands` slot) to a handler, and `host.turn` (`initialize`, `before_model`, `after_turn`) applies a bounded policy to each agent turn. Their handlers must also appear in `handlers`. Use them only when the user asks for that behavior, and follow the official Goals plugin as a model. With `conversation.message.read`, the `initialize` input also carries `message`: the user's text for that turn, capped at 4 KiB, or `null` when the turn has no user text.
