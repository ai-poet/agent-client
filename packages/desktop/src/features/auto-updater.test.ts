import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const appMock = vi.hoisted(() => ({
  isPackaged: true,
}));

const updaterMock = vi.hoisted(() => ({
  autoDownload: false,
  autoInstallOnAppQuit: false,
  autoRunAppAfterInstall: false,
  allowPrerelease: false,
  channel: null as string | null,
  allowDowngrade: true,
  setFeedURL: vi.fn(),
  on: vi.fn(),
  checkForUpdates: vi.fn(),
  downloadUpdate: vi.fn(),
  quitAndInstall: vi.fn(),
}));

vi.mock("electron", () => ({
  app: appMock,
}));

vi.mock("electron-updater", () => ({
  autoUpdater: updaterMock,
}));

async function loadModule(env: NodeJS.ProcessEnv = {}) {
  vi.resetModules();
  for (const key of [
    "PASEO_APP_NAME",
    "PASEO_DESKTOP_UPDATE_URL",
    "PASEO_DESKTOP_ICON_PNG",
    "PASEO_DESKTOP_ICON_MAC",
    "PASEO_DESKTOP_ICON_WIN",
    "PASEO_DESKTOP_ICON_LINUX",
  ]) {
    delete process.env[key];
  }
  Object.assign(process.env, env);
  return import("./auto-updater");
}

describe("auto updater helpers", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("resolves platform-specific channel files", async () => {
    const { channelFileForPlatform } = await loadModule();

    expect(channelFileForPlatform("stable", "win32")).toBe("latest.yml");
    expect(channelFileForPlatform("stable", "darwin")).toBe("latest-mac.yml");
    expect(channelFileForPlatform("stable", "linux")).toBe("latest-linux.yml");
    expect(channelFileForPlatform("beta", "win32")).toBe("beta.yml");
    expect(channelFileForPlatform("beta", "darwin")).toBe("beta-mac.yml");
    expect(channelFileForPlatform("beta", "linux")).toBe("beta-linux.yml");
  });

  it("parses latest version from electron-builder update metadata", async () => {
    const { parseLatestVersionFromUpdateInfo } = await loadModule();

    expect(parseLatestVersionFromUpdateInfo("version: 1.2.3\npath: app.exe\n")).toBe("1.2.3");
    expect(parseLatestVersionFromUpdateInfo("version: '1.2.3-beta.4'\n")).toBe("1.2.3-beta.4");
    expect(parseLatestVersionFromUpdateInfo("path: app.exe\n")).toBeNull();
  });

  it("compares stable and prerelease versions", async () => {
    const { isRemoteVersionNewer } = await loadModule();

    expect(isRemoteVersionNewer("1.2.4", "1.2.3")).toBe(true);
    expect(isRemoteVersionNewer("v1.2.3", "1.2.3")).toBe(false);
    expect(isRemoteVersionNewer("1.2.3-beta.2", "1.2.3-beta.1")).toBe(true);
    expect(isRemoteVersionNewer("1.2.3-beta.1", "1.2.3")).toBe(false);
  });

  it("fetches MinIO metadata with normalized URL and no-cache query", async () => {
    const { fetchLatestUpdateInfo } = await loadModule();
    const fetcher = vi.fn(async () => ({
      ok: true,
      status: 200,
      text: async () => "version: 1.2.3\n",
    })) as unknown as typeof fetch;

    const result = await fetchLatestUpdateInfo({
      updateUrl: "minio.cyberspirit.io/releases",
      releaseChannel: "beta",
      platform: "darwin",
      fetcher,
    });

    expect(result?.version).toBe("1.2.3");
    expect(result?.channelFile).toBe("beta-mac.yml");
    const requestedUrl = fetcher.mock.calls[0]?.[0] as URL;
    expect(requestedUrl.origin).toBe("https://minio.cyberspirit.io");
    expect(requestedUrl.pathname).toBe("/releases/beta-mac.yml");
    expect(requestedUrl.searchParams.get("noCache")).toBeTruthy();
  });
});

describe("MinIO auto updater flow", () => {
  beforeEach(() => {
    appMock.isPackaged = true;
    updaterMock.setFeedURL.mockReset();
    updaterMock.on.mockReset();
    updaterMock.checkForUpdates.mockReset();
    updaterMock.downloadUpdate.mockReset();
    updaterMock.quitAndInstall.mockReset();
    updaterMock.autoDownload = false;
    updaterMock.autoInstallOnAppQuit = false;
    updaterMock.autoRunAppAfterInstall = false;
    updaterMock.allowPrerelease = false;
    updaterMock.channel = null;
    updaterMock.allowDowngrade = true;
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("disables packaged app updates when the brand has no update URL", async () => {
    const { checkForAppUpdate } = await loadModule({ PASEO_APP_NAME: "Paseo" });

    const result = await checkForAppUpdate({
      currentVersion: "1.0.0",
      releaseChannel: "stable",
    });

    expect(result).toMatchObject({
      hasUpdate: false,
      readyToInstall: false,
      currentVersion: "1.0.0",
      latestVersion: "1.0.0",
      disabledReason: "Desktop updates are not configured for this brand.",
    });
    expect(updaterMock.setFeedURL).not.toHaveBeenCalled();
    expect(updaterMock.checkForUpdates).not.toHaveBeenCalled();
  });

  it("uses CyberAICoding's default MinIO update URL", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({
        ok: true,
        status: 200,
        text: async () => "version: 1.0.0\n",
      })),
    );
    const { checkForAppUpdate } = await loadModule({ PASEO_APP_NAME: "CyberAICoding" });

    await checkForAppUpdate({ currentVersion: "1.0.0", releaseChannel: "stable" });

    expect(updaterMock.setFeedURL).toHaveBeenCalledWith({
      provider: "generic",
      url: "https://minio.cyberspirit.io/",
    });
    expect(updaterMock.checkForUpdates).not.toHaveBeenCalled();
  });

  it("uses MinIO metadata to avoid updater checks when already current", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({
        ok: true,
        status: 200,
        text: async () => "version: 1.0.0\n",
      })),
    );
    const { checkForAppUpdate } = await loadModule({
      PASEO_DESKTOP_UPDATE_URL: "https://updates.example.com/app/",
    });

    const result = await checkForAppUpdate({
      currentVersion: "1.0.0",
      releaseChannel: "stable",
    });

    expect(result).toMatchObject({
      hasUpdate: false,
      latestVersion: "1.0.0",
      disabledReason: null,
    });
    expect(updaterMock.checkForUpdates).not.toHaveBeenCalled();
  });

  it("delegates to electron-updater when MinIO metadata is newer", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({
        ok: true,
        status: 200,
        text: async () => "version: 1.0.1\n",
      })),
    );
    updaterMock.checkForUpdates.mockResolvedValue({
      updateInfo: {
        version: "1.0.1",
        files: [],
        path: "",
        sha512: "",
        releaseDate: "2026-05-13T00:00:00.000Z",
        releaseNotes: "Fixes",
      },
    });
    const { checkForAppUpdate } = await loadModule({
      PASEO_DESKTOP_UPDATE_URL: "https://updates.example.com/app/",
    });

    const result = await checkForAppUpdate({
      currentVersion: "1.0.0",
      releaseChannel: "stable",
    });

    expect(updaterMock.checkForUpdates).toHaveBeenCalled();
    expect(result).toMatchObject({
      hasUpdate: true,
      latestVersion: "1.0.1",
      body: "Fixes",
      date: "2026-05-13T00:00:00.000Z",
      disabledReason: null,
    });
  });
});
