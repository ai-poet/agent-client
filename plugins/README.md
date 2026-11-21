# Sailry plugins

Official Sailry plugin packages and installable SDK examples. The application,
Node runtime and [SDK contract](https://github.com/ai-poet/agent-client/blob/main/sdk/plugins.md)
live in [sailry-harness](https://github.com/ai-poet/agent-client).

Each top-level package is an ordinary Agent Plugins package with Sailry's v1
extension manifest. `examples/` contains optional developer examples. Package
sources are maintained here only; the application pins this repository as a Git
submodule and embeds its reviewed package snapshot for offline installation.
Startup adds missing default packages and refreshes newer bundled versions,
preserving configuration and enable choices. It does not reinstall packages
removed by the user.

## Versions and immutable packages

New packages start at `0.1.0`. Increase the semantic version when package content
changes; ordinary updates increment the patch version, such as `0.1.0` to
`0.1.1`. Update detection requires strictly newer SemVer precedence. Same-version
content changes, build metadata alone and older versions do not offer an update.
Package versions are independent of the API contract. The application currently
supports Sailry extension API `v1`; package declarations use the contract supported
by their target application.
Immutable content digests still verify installed contents and preserve captured
resources and sent-turn references. Updates apply to new admission; historical
captures keep their recorded package. Standalone imported skills retain their
repository-commit update identity rather than adopting plugin release versions.

Declare `extensions."dev.sailry.platform".min_sailry_version` when a package
needs a new host module, export, extension field or permission. Set it to the first
application version with that capability, such as `0.1.0-alpha.4`. It is an exact
SemVer floor, not a version range; omission imposes no minimum. This declaration
changes package content and requires a version bump.

Package marketplace metadata uses the existing v1 extension: optional `category`
(`development`, `documents`, `data`, `automation`, `entertainment`) and `tint`
(`blue`, `violet`, `teal`, `amber`, `gray`). Tool `display.locales` supplies names
for the ten application locales. Skill frontmatter uses string metadata keys
`display-name` and `display-name.<locale>`; model descriptions and skill bodies
remain separate from these display names.

Release notes live in `CHANGELOG.md`, with headings such as `## 0.1.2` matching
the manifest version exactly. Optional `CHANGELOG.<locale>.md` files use the same
headings. The generator includes only the selected version's plain text, alongside
real `skills/*/SKILL.md` frontmatter in the shared catalog. Missing notes are
omitted. New notes count as package content and require the usual version bump.
`catalog.py --check` reports existing translations that lack the selected version;
those locales retain the current English fallback until their notes are updated.

Each catalog package also includes `versions`, ordered by descending SemVer
precedence. A version records one immutable package-content commit, its declared
`api_version` and `min_sailry_version`, and release notes from that commit.
`api_version` is omitted for packages without the Sailry extension; declared API
strings are preserved even when the current application does not support them.
Build metadata does not affect precedence; equal-precedence records retain
newest-commit order. The current `manifest` and notes remain available alongside
the history.

Catalog generation needs complete Git history. Commit package changes first,
then run the generator and commit `catalog.json` separately so every version
points to an existing commit. Unrelated repository or catalog-only commits do not
change package history. Missing historical translations are omitted.

The committed `HEAD:catalog.json` is the publication record. Generation preserves
its exact version-to-commit mappings, including historical identities accepted
before the immutable publication rule. Past same-version edits cannot be repaired
by selecting an earlier or later commit. Previously retired version lineages stay
retired. A new version must have exactly one package tree across its history and
records its first content commit. It must also have distinct SemVer precedence:
changing build metadata cannot publish another package under the same release
precedence. Already recorded build ties retain their identities. All package
commits introduced since the last catalog publication are checked, so a
same-version edit is rejected even if a later commit increases the version.
Correct such an unpublished commit before
publishing; a published package fix needs a new version. Generation also compares
the current package tree with its recorded version. Do not edit recorded commit
identities by hand: the committed index is the reviewed authority, not a signed
external release registry.

`catalog-withdrawn.json` lists versions excluded from recommendations and
installations when their compatibility metadata or contents are unsuitable.
Withdrawn records remain in `versions` with `withdrawn: true`, retaining their
original commit and metadata. The latest version can also be withdrawn. Versions
omitted by older withdrawal catalogs recover their last recorded identities from
committed catalog history. Withdrawals leave installed packages and captured
resources intact; the next compatible historical version remains available to
older applications.

## Working with the application submodule

This repository has its own Git history. The application's `plugins/` directory
is a Git submodule that records a reviewed commit, not a symbolic link. Clone the
application with its pinned submodules:

```sh
git clone --recurse-submodules https://github.com/ai-poet/agent-client.git
```

For an existing application checkout, run this from its root after pulling parent
changes, preserving any local work first:

```sh
git submodule update --init --recursive
```

A submodule checkout commonly has a detached HEAD. Before committing plugin
changes, select an existing owned branch or create one from the pinned commit:

```sh
git -C plugins status --short
git -C plugins switch -c feature/plugin-change
```

Commit plugin changes inside this repository, then commit the updated `plugins/`
pointer separately in the application repository. A parent commit or ordinary
push does not commit or publish plugin files. When publication is requested,
push the plugin commit first, then push the parent commit that references it.
This keeps the recorded plugin snapshot available to recursive checkouts.

After both repositories have their changes committed on branches, Git can push
required plugin commits before the parent in one command:

```sh
git push --recurse-submodules=on-demand
```

This does not commit pending file changes. Repository-local
`push.recurseSubmodules=on-demand` can make it the default for future pushes.

## Checks and catalog

Use Python 3.12 or newer and Node.js 22.22.1:

```sh
python3 -m unittest discover -s scripts/tests -v
python3 scripts/check-names.py
node --experimental-vm-modules scripts/check-packages.mjs
python3 scripts/test.py
python3 scripts/catalog.py --check
```

CI also checks workflow syntax. Package checks cover all official packages and
examples, v1 contracts, declared assets, static imports and JavaScript syntax
without executing plugin initialization. Behavior tests include controllers,
host handlers, games and localized UI contracts; real Node integration remains
in the application repository.

Production moejs and SDK cases use the application's `internal/node/plugins`
test binary. Its `TestMain` handles fixture requests when the runner sets
`SAILRY_PLUGIN_TEST_HOST`. Build that host explicitly from the application checkout,
then pass its absolute path to the package runner:

```sh
cd /path/to/sailry-harness
unset GOROOT
export GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0
mkdir -p .runtime/release-review
go test -c -o .runtime/release-review/plugin-test-host ./internal/node/plugins
python3 plugins/scripts/test.py --runtime-host "$PWD/.runtime/release-review/plugin-test-host"
```

Package tests import `scripts/test-host.mjs` and own their inputs and assertions.
The helper loads declared host modules and supplies explicit typed service
replies; it does not start a Node, access a profile, or prove UI or transport
behavior. Without `--runtime-host`, these cases are reported as skipped rather
than falling back to a mock. Ordinary package tests keep using the same runner.

Keep function and test names concise. Files, modules and classes supply context;
do not repeat their names as prefixes or add groups just to shorten labels.
The shared naming check allows seven semantic words in identifiers and twelve
in test labels, excluding required Python `test_` prefixes and generated source.

CI regenerates `catalog.json` from package manifests and publishes changes after
successful checks on `main`. Run `python3 scripts/catalog.py` to review index
changes locally. The marketplace and update checks read the same versions and
descriptions from this index. Check the regenerated index with
`python3 scripts/catalog.py --check`. Explicit installations and updates resolve
a package path to an immutable Git commit through the execution Node; clients do
not execute Git or receive credentials.

## License and provenance

Sailry-owned sources use [Apache-2.0](LICENSE). Package-local notices and licenses
remain authoritative for imported resources. Sources were extracted from the
reviewed sailry-harness commit `609c544`; original package provenance and notices
are preserved. No profiles, credentials or private development history are part
of this repository.
