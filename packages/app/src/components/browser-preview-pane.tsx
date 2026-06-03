// This file exists for TypeScript resolution.
// The actual implementations are in:
// - browser-preview-pane.native.tsx (iOS/Android)
// - browser-preview-pane.web.tsx (Web/Electron)
// Metro's platform-specific extensions will pick the right one at runtime.

export * from "./browser-preview-pane.native";
