# macOS frame and Metal resource lifetime

A clean native window previously dispatched an application callback on every
CVDisplayLink tick. The window now parks those main-queue callbacks after drawing
when no invalidation, presentation, next-frame callback or sustained high-rate
input remains. Content invalidations, animation callbacks and input rearm the
existing display clock. Activation/occlusion/resize keep their existing source
lifecycle. The immortal per-display link registry and its teardown protections
remain; its lightweight timing thread still runs while subscribed.

Metal drawing has a per-frame Objective-C autorelease pool. Backdrop blur uses
up to four cached MPS Gaussian kernels and four scratch texture pairs instead
of repeatedly replacing one pair/kernel when a composer and menu alternate.
Texture extents use 64-pixel buckets, clipped to the drawable; retained pairs
are limited to two drawable pairs or 32 MiB, except one required larger pair.
Shrinking the window discards incompatible extents. Parking a window drops
pairs unused by its last frame, including obsolete menu/resize allocations,
and path intermediates absent from that frame. In-flight command buffers retain
the resources they need. Current surfaces retain their resources for resumption.

Gaussian sigma, three-sigma snapshot padding, edge clamping, shaders, clipping,
corner geometry, MSAA, animation clocks and refresh rates are unchanged.

## Validation

On an Apple Silicon Mac with native Metal, 207 GPUI library tests and 15 macOS
library tests passed. Coverage includes parking a clean window, waking on entity
notification and resize, delivering every callback in a three-frame animation,
blur gradients/sharp edges/window edges, alternating different-sized blurs,
byte-identical cold/warm rendering, texture/kernel bounds and menu-close trimming.
The standalone embedded test platform can now run a launch callback, enabling
native offscreen UI replays with real animation clocks.

```sh
cargo test --release -p gpui -p gpui_macos \
  --features gpui_macos/runtime_shaders --lib
```

This extracted repository's existing GPUI SVG tests require the IBM Plex Sans
and Lilex font fixtures from upstream Zed under `assets/fonts/`. They were supplied
locally for this run. `runtime_shaders` supports hosts with Command Line Tools
but without the offline Metal compiler.

The test host's display was locked. Offscreen tests exercise native Metal but do
not validate foreground presentation or establish whole-app CPU/memory savings.
Unlocked checks of typing, scrolling, stream completion, animation, menus,
resizing, occlusion and display reconnection remain before merge.
