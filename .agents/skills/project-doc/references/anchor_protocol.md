# Anchor protocol

## Navigation

- [Syntax](#syntax)
- [Comment forms](#comment-forms)
- [Placement](#placement)
- [Reading behavior](#reading-behavior)
- [Change rules](#change-rules)
- [Checks](#checks)

## Syntax

A code anchor is a fixed, searchable marker on a single line:

```text
@project-doc docs/<path>.md#<section_id>
```

`docs/` is the default documentation root. If the project uses another root, use its actual path relative to the repository root. Absolute paths, `../`, editor URIs, remote repository URLs, and paths relative to the current file break when the environment or file location changes.

Each anchor follows these rules:

- Write the marker as lowercase `@project-doc`.
- Use a repository-root-relative path to an indexed Markdown document.
- Use lowercase `snake_case` for filenames and directories in the path.
- Match `section_id` to `[a-z][a-z0-9_]*`.
- Put one target on each line. Use multiple anchor lines when an entry point depends on several sections.

Place an HTML ID before the target heading:

```markdown
<a id="account_selection"></a>
## Account selection
```

Use this ID in the anchor. Markdown renderers derive heading slugs from wording. Improve heading wording as needed; change a stable ID when the section's subject changes.

## Comment forms

Wrap the anchor in the target language's comment syntax. Keep the anchor on its own comment line:

```go
// @project-doc docs/architecture/request_routing.md#account_selection
func selectAccount(...) {
```

```typescript
// @project-doc docs/interfaces/session_contract.md#refresh_rules
export async function refreshSession(...) {
```

```python
# @project-doc docs/domains/billing.md#balance_invariant
def apply_charge(...):
```

```sql
-- @project-doc docs/domains/usage.md#aggregation_boundary
CREATE TABLE usage_records (...);
```

```html
<!-- @project-doc docs/interfaces/authentication.md#login_flow -->
<LoginForm />
```

An anchor is a protocol marker. Put additional explanations on a separate line in the repository's required comment language.

## Placement

Place anchors where readers need documentation to understand the code, in this order of preference:

1. A handwritten entry point for a subsystem or cross-module flow.
2. A type, interface, or state machine that owns a core invariant.
3. A code entry point that implements a public contract.
4. The handwritten code nearest the rule's implementation when there is no single entry point.

Place the anchor beside its symbol or configuration block. Anchors grouped at the file header obscure which code they describe. Several actual entry points may reference one section. Copying the anchor into every related file creates many references to maintain.

Skip these locations by default:

- Generated code, vendor code, minified files, and lockfiles.
- Formats without comments, such as JSON.
- Migration files, which record historical steps while current rules live elsewhere.
- Ordinary tests that verify the same rule as production code.

If the target file cannot contain comments, place the anchor in its generation source, schema, loader, validator, or nearest handwritten entry point. A test may carry an anchor when the test itself defines a cross-module contract and no production entry point is more suitable.

## Reading behavior

When an agent encounters an anchor in code:

1. Resolve its repository-root-relative path and section ID.
2. Before explaining, reviewing, or changing the code, read or reuse the target section under [Reading and context reuse](../SKILL.md#reading-and-context-reuse). Reuse it when encountering the same anchor again.
3. Read the scope and direct dependencies needed to understand the section. Read the whole document as needed.
4. Record an invalid path or ID as a documentation/code inconsistency.

Report broken anchors and their locations in read-only tasks. When changes are authorized, repair the anchor, documentation, or both. Choose a replacement target from evidence. Guessing a section from a similar name can direct readers to the wrong rule.

## Change rules

- Rewording a heading: preserve its ID and code anchors while the subject stays the same.
- Reorganizing sections: move the existing ID with the same subject. After a split, create IDs for new content and assess each old anchor's destination.
- Replacing section content: create a new ID, update all code anchors, then remove obsolete sections once they have no references.
- Moving code: move the anchor with the code implementing its rule and remove it from the old location.
- Moving or renaming documents: update all anchors, indexes, and ordinary links in the same change.
- Merging documents: map every old anchor to a specific section in the new document.
- Removing a feature: preserve any valid knowledge first, then remove related anchors, sections, and index entries.

## Checks

List anchors with `rg -n '@project-doc'` and check each one:

- The path exists and belongs to the library.
- The target contains an exact matching `<a id="..."></a>`.
- IDs are unique within the file.
- Paths and IDs follow naming rules.
- The anchor sits beside handwritten code that implements the rule.
- Old paths and IDs have been removed after moves or deletions.

Search backward from referenced sections to their code references and confirm that important entry points still have anchors after refactoring. Assess coverage by whether those entry points are anchored.
