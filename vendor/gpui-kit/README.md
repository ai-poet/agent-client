# Waku UI (`vendor/gpui-kit`)

Waku's reusable GPUI component crate. The Rust package is named `client-ui` to
avoid confusion with the Longbridge GPUI Kit reference checkout in the repository
root.

## Ownership

- `vendor/gpui`: the only GPUI engine and its platform/rendering patches.
- `vendor/gpui-kit`: interaction behavior, reusable components and their tests.
- Waku app: provider/session state, persistence, localization, assets and domain
  composition.

The component crate must not depend on the root `waku` crate or Waku's protocol
and client crates. Domain data enters as props or immutable snapshots; user
intent leaves through callbacks. `UiTheme` is a value passed by the app, not a
second global theme.

## Migration and porting

Move one component at a time and keep the existing `src/ui` API as an adapter
until call sites can migrate without behavior changes. Read `docs/performance.md`
before moving lists, motion, scrollbars or anything reached by a streaming frame.

External implementations are ported selectively, never imported as a second
GPUI stack. Record closely adapted source, its upstream revision and license in
`NOTICE.md` and the repository's root `NOTICE.md`.

## Checks

```bash
cargo test -p client-ui
cargo check -p client-ui
```
