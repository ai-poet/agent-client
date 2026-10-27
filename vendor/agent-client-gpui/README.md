# zui

Zeron's fork of [GPUI](https://gpui.rs), Zed's GPU-accelerated UI framework, extracted
into a standalone workspace so consumers don't have to clone the full Zed repository.

## Provenance

Extracted from [`wingleeio/zed`](https://github.com/wingleeio/zed) at commit
`e2ddcc6805f8c5088e62a60dfe517abcccd61a9a` — upstream
[`zed-industries/zed`](https://github.com/zed-industries/zed) `f14fea9` plus the
comet patch line (line-wrap closing punctuation, EdgeFade, BackdropBlur, wgpu
frosted-glass rasterization, GPU memory bounding, `ImageSource::evict`,
transparent-window destination alpha, macOS 26 UnderWindowBackground blur, and
intrinsic-aspect-only-when-auto image sizing). See the comment block above the
`gpui` dependency in comet's `Cargo.toml` history for the full per-commit log.

The workspace contains `gpui`, `gpui_platform`, `gpui_tokio`, and their in-tree
dependency closure. Crate directory paths (`crates/<name>`, `tooling/perf`) match
Zed's layout so upstream patches apply with the same `-p` level.

## Porting upstream changes

This repo has no shared git history with Zed. To port an upstream commit:

```sh
git -C ../zed format-patch -1 <sha> --stdout | git apply --3way
```

Paths line up as long as the touched crates exist here.

## License

`gpui` and most crates here are Apache-2.0 (see `LICENSE-APACHE`). The `path`
crate remains GPL-3.0-or-later (see `LICENSE-GPL`), matching its licensing in
the Zed repository. Individual dependencies retain their own licenses.

The optional sum-tree profiling annotations and the `ztracing`,
`ztracing_macro`, and `zlog` crates have been removed. Sum-tree tests use
`env_logger` for logging.
