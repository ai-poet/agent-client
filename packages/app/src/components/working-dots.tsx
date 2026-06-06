// This file exists for TypeScript resolution.
// The actual implementations are in:
// - working-dots.native.tsx (iOS/Android)
// - working-dots.web.tsx (Web/Electron)
// Metro's platform-specific extensions will pick the right one at runtime.

export * from "./working-dots.native";
