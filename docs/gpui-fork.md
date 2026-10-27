# Local GPUI fork

Waku uses a local GPUI workspace so the desktop can combine the `waku-webview`
integration with Zeron's renderer work. The two `patch` entries in `Cargo.toml`
resolve `gpui` and `gpui_platform` to `../../agent-client-gpui/crates/`.

## Current base

- Repository: <https://github.com/zeronsh/zui>
- Branch upstream: `main`
- Base commit: `361ff20b919fdfb80199ee6d9aed725300cb7d54`
- Local integration branch: `waku-local`
- Additional Waku patches:
  - `e99616cdd466` (`gpui: Add resizable and minimizable state to Window`,
    local commit `00179677e60d5a65858c8545136077f1df7e78eb`),
    limited to `crates/gpui/src/{platform,window}.rs` and
    `crates/gpui_macos/src/window.rs`. The Zed-only `platform_title_bar` and
    `workspace` parts do not apply to this standalone workspace.

Zui is a standalone extraction of GPUI from `wingleeio/zed`. Its `main` branch
already includes the native-content overlay planes, per-pixel edge fades,
rounded clipping, and macOS idle-frame parking needed by Waku. In particular,
the `codex/idle-resource-stability` line is already contained in this base.
This is why the local workspace is based on zui rather than on a full Zed
checkout.

Zui also removes the GPL tracing crates from the GPUI dependency graph. The
resulting `Cargo.lock` differences are expected; keep them when updating the
local fork.

## Porting Waku or Zed changes

Zui and Zed do not share Git history. Do not merge a Zed branch into this
repository wholesale. Port only a reviewed, needed commit, following zui's own
convention:

```bash
git -C ../zed format-patch -1 <sha> --stdout | git apply --3way
```

The crate paths match Zed's layout, so the same patch level applies. Review
every conflict and record the source commit here. Do not port a native-overlay
change merely because it is absent by title: zui's overlay implementation has
its own history and may already contain the equivalent behavior.

## Verification

From `client/`, run:

```bash
cargo metadata --locked --no-deps
cargo fetch --locked
cargo check --locked
```

Run focused zui tests for any additional patch. Keep the local integration
branch reproducible and do not replace it with a machine-specific absolute
path in `Cargo.toml`.
