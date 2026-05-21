import { beforeEach, describe, expect, it, vi } from "vitest";

const asyncStorageMock = vi.hoisted(() => ({
  getAllKeys: vi.fn<() => Promise<string[]>>(),
  multiRemove: vi.fn<(_: string[]) => Promise<void>>(),
}));

const invokeDesktopCommandMock = vi.hoisted(() => vi.fn<(_: string) => Promise<unknown>>());
const hostRuntimeStoreMock = vi.hoisted(() => ({
  reset: vi.fn<() => Promise<void>>(),
}));

vi.mock("@react-native-async-storage/async-storage", () => ({
  default: asyncStorageMock,
}));

vi.mock("@/desktop/electron/invoke", () => ({
  invokeDesktopCommand: invokeDesktopCommandMock,
}));

vi.mock("@/runtime/host-runtime", () => ({
  getHostRuntimeStore: () => hostRuntimeStoreMock,
}));

vi.mock("@/constants/platform", () => ({
  isWeb: true,
}));

describe("resetPaseoEnvironment", () => {
  beforeEach(() => {
    vi.resetModules();
    asyncStorageMock.getAllKeys.mockReset();
    asyncStorageMock.multiRemove.mockReset();
    invokeDesktopCommandMock.mockReset();
    hostRuntimeStoreMock.reset.mockReset();
    asyncStorageMock.getAllKeys.mockResolvedValue([]);
    asyncStorageMock.multiRemove.mockResolvedValue();
    invokeDesktopCommandMock.mockResolvedValue({ success: true });
    hostRuntimeStoreMock.reset.mockResolvedValue();
  });

  it("resets desktop state, clears Paseo storage, deletes attachments DB, and reloads", async () => {
    const deleteDatabase = vi.fn(() => ({
      onsuccess: null as (() => void) | null,
      onerror: null as (() => void) | null,
      onblocked: null as (() => void) | null,
    }));
    const location = { assign: vi.fn() };
    Object.defineProperty(globalThis, "indexedDB", {
      configurable: true,
      value: { deleteDatabase },
    });
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: { location },
    });
    asyncStorageMock.getAllKeys.mockResolvedValue([
      "@paseo:daemon-registry",
      "paseo-drafts",
      "workspace-layout-state",
      "unrelated-key",
    ]);

    const { resetPaseoEnvironment } = await import("./reset-paseo-environment");
    const resetPromise = resetPaseoEnvironment();
    await vi.waitFor(() => {
      expect(deleteDatabase).toHaveBeenCalledWith("paseo-attachment-bytes");
    });
    const request = deleteDatabase.mock.results[0]?.value;
    request.onsuccess?.();
    await resetPromise;

    expect(invokeDesktopCommandMock).toHaveBeenCalledWith("reset_paseo_environment");
    expect(hostRuntimeStoreMock.reset).toHaveBeenCalledTimes(1);
    expect(asyncStorageMock.multiRemove).toHaveBeenCalledWith([
      "@paseo:daemon-registry",
      "paseo-drafts",
      "workspace-layout-state",
    ]);
    expect(location.assign).toHaveBeenCalledWith("/");
  });
});
