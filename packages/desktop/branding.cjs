function trimToNull(value) {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

const DESKTOP_UPDATE_URL_BY_BRAND = {
  cyberaicoding: "https://minio.cyberspirit.io/",
  cheaprouter: "https://file.masterwordai.com/",
};

function normalizeUrlToNull(value) {
  const trimmed = trimToNull(value);
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

function resolveDesktopUpdateUrl(env, appName) {
  return (
    normalizeUrlToNull(env.PASEO_DESKTOP_UPDATE_URL) ??
    normalizeUrlToNull(DESKTOP_UPDATE_URL_BY_BRAND[appName.toLowerCase()]) ??
    null
  );
}

function resolveDesktopBrandingFromEnv(env = process.env) {
  const appName = trimToNull(env.PASEO_APP_NAME) ?? "Paseo";
  return {
    appName,
    desktopAppId: trimToNull(env.PASEO_DESKTOP_APP_ID) ?? "sh.paseo.desktop",
    desktopIconPng: trimToNull(env.PASEO_DESKTOP_ICON_PNG) ?? "assets/icon.png",
    desktopIconMac: trimToNull(env.PASEO_DESKTOP_ICON_MAC) ?? "assets/icon.icns",
    desktopIconWin: trimToNull(env.PASEO_DESKTOP_ICON_WIN) ?? "assets/icon.ico",
    desktopIconLinux: trimToNull(env.PASEO_DESKTOP_ICON_LINUX) ?? "assets",
    desktopUpdateUrl: resolveDesktopUpdateUrl(env, appName),
  };
}

module.exports = {
  resolveDesktopBrandingFromEnv,
  normalizeUrlToNull,
};
