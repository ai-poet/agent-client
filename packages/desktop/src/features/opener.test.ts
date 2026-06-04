import { ipcMain, shell } from "electron";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { isAllowedExternalUrl, registerOpenerHandlers } from "./opener";
import { __setDesktopLocaleForTests } from "../i18n/desktop-i18n";

vi.mock("electron", () => ({
  ipcMain: { handle: vi.fn() },
  shell: { openExternal: vi.fn() },
}));

function getRegisteredOpenUrlHandler(): (_event: unknown, url: unknown) => Promise<void> {
  registerOpenerHandlers();
  const handler = vi.mocked(ipcMain.handle).mock.calls.find(([channel]) => {
    return channel === "paseo:opener:openUrl";
  })?.[1];
  if (typeof handler !== "function") {
    throw new Error("open URL handler was not registered");
  }
  return handler as (_event: unknown, url: unknown) => Promise<void>;
}

describe("desktop opener", () => {
  beforeEach(() => {
    vi.mocked(ipcMain.handle).mockReset();
    vi.mocked(shell.openExternal).mockReset();
  });

  afterEach(() => {
    __setDesktopLocaleForTests(null);
  });

  it("allows only http and https external URLs", () => {
    expect(isAllowedExternalUrl("https://example.com/path")).toBe(true);
    expect(isAllowedExternalUrl("http://localhost:8081")).toBe(true);
    expect(isAllowedExternalUrl("file:///etc/passwd")).toBe(false);
    expect(isAllowedExternalUrl("javascript:alert(1)")).toBe(false);
    expect(isAllowedExternalUrl("paseo://settings")).toBe(false);
    expect(isAllowedExternalUrl("/relative/path")).toBe(false);
    expect(isAllowedExternalUrl(null)).toBe(false);
  });

  it("opens allowed URLs through Electron shell", async () => {
    const handler = getRegisteredOpenUrlHandler();

    await handler({}, "https://example.com");

    expect(shell.openExternal).toHaveBeenCalledWith("https://example.com");
  });

  it("rejects blocked URLs before invoking Electron shell", async () => {
    __setDesktopLocaleForTests("zh");
    const handler = getRegisteredOpenUrlHandler();

    await expect(handler({}, "file:///etc/passwd")).rejects.toThrow("不支持此外部 URL。");

    expect(shell.openExternal).not.toHaveBeenCalled();
  });
});
