import { readFileSync } from "node:fs";
import path from "node:path";

export interface DesktopBranding {
  appName: string;
  desktopAppId: string;
  desktopIconPng: string;
  desktopIconMac: string;
  desktopIconWin: string;
  desktopIconLinux: string;
  desktopUpdateUrl: string | null;
}

const DEFAULT_DESKTOP_BRANDING: DesktopBranding = {
  appName: "Paseo",
  desktopAppId: "sh.paseo.desktop",
  desktopIconPng: "assets/icon.png",
  desktopIconMac: "assets/icon.icns",
  desktopIconWin: "assets/icon.ico",
  desktopIconLinux: "assets",
  desktopUpdateUrl: null,
};

const DESKTOP_UPDATE_URL_BY_BRAND: Record<string, string> = {
  cyberaicoding: "https://minio.cyberspirit.io/",
  cheaprouter: "https://file.masterwordai.com/",
};

function trimToNull(value: string | undefined): string | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export function normalizeDesktopUpdateUrl(value: string | undefined | null): string | null {
  const trimmed = trimToNull(value ?? undefined);
  if (!trimmed) {
    return null;
  }

  const withProtocol = /^[a-z][a-z\d+.-]*:\/\//i.test(trimmed) ? trimmed : `https://${trimmed}`;

  try {
    const url = new URL(withProtocol);
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      return null;
    }
    url.hash = "";
    url.search = "";
    if (!url.pathname.endsWith("/")) {
      url.pathname = `${url.pathname}/`;
    }
    return url.toString();
  } catch {
    return null;
  }
}

function readStringField(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : null;
}

function readCompiledDesktopBranding(): Partial<DesktopBranding> {
  try {
    const raw = readFileSync(path.join(__dirname, "desktop-branding.json"), "utf-8");
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    return {
      appName: readStringField(parsed.appName) ?? undefined,
      desktopAppId: readStringField(parsed.desktopAppId) ?? undefined,
      desktopIconPng: readStringField(parsed.desktopIconPng) ?? undefined,
      desktopIconMac: readStringField(parsed.desktopIconMac) ?? undefined,
      desktopIconWin: readStringField(parsed.desktopIconWin) ?? undefined,
      desktopIconLinux: readStringField(parsed.desktopIconLinux) ?? undefined,
      desktopUpdateUrl: readStringField(parsed.desktopUpdateUrl),
    };
  } catch {
    return {};
  }
}

export function getDesktopBranding(): DesktopBranding {
  const compiled = readCompiledDesktopBranding();
  const appName =
    trimToNull(process.env.PASEO_APP_NAME) ?? compiled.appName ?? DEFAULT_DESKTOP_BRANDING.appName;
  const defaultUpdateUrl = DESKTOP_UPDATE_URL_BY_BRAND[appName.toLowerCase()] ?? null;
  return {
    appName,
    desktopAppId:
      trimToNull(process.env.PASEO_DESKTOP_APP_ID) ??
      compiled.desktopAppId ??
      DEFAULT_DESKTOP_BRANDING.desktopAppId,
    desktopIconPng:
      trimToNull(process.env.PASEO_DESKTOP_ICON_PNG) ??
      compiled.desktopIconPng ??
      DEFAULT_DESKTOP_BRANDING.desktopIconPng,
    desktopIconMac:
      trimToNull(process.env.PASEO_DESKTOP_ICON_MAC) ??
      compiled.desktopIconMac ??
      DEFAULT_DESKTOP_BRANDING.desktopIconMac,
    desktopIconWin:
      trimToNull(process.env.PASEO_DESKTOP_ICON_WIN) ??
      compiled.desktopIconWin ??
      DEFAULT_DESKTOP_BRANDING.desktopIconWin,
    desktopIconLinux:
      trimToNull(process.env.PASEO_DESKTOP_ICON_LINUX) ??
      compiled.desktopIconLinux ??
      DEFAULT_DESKTOP_BRANDING.desktopIconLinux,
    desktopUpdateUrl:
      normalizeDesktopUpdateUrl(process.env.PASEO_DESKTOP_UPDATE_URL) ??
      normalizeDesktopUpdateUrl(compiled.desktopUpdateUrl) ??
      normalizeDesktopUpdateUrl(defaultUpdateUrl) ??
      DEFAULT_DESKTOP_BRANDING.desktopUpdateUrl,
  };
}
