import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const rootPackageJson = JSON.parse(readFileSync(new URL("../package.json", import.meta.url)));
const cheapRouterScript = readFileSync(
  new URL("./run-with-cheaprouter-brand.mjs", import.meta.url),
  "utf8",
);
const cyberAiCodingScript = readFileSync(
  new URL("./run-with-cyberaicoding-brand.mjs", import.meta.url),
  "utf8",
);

describe("CheapRouter brand script", () => {
  it("is exposed from root package scripts", () => {
    expect(rootPackageJson.scripts["with:cheaprouter"]).toBe(
      "node scripts/run-with-cheaprouter-brand.mjs",
    );
    expect(rootPackageJson.scripts["build:desktop:cheaprouter"]).toBe(
      "node scripts/run-with-cheaprouter-brand.mjs npm run build:desktop",
    );
  });

  it("sets CheapRouter app, icon, desktop, and cloud endpoint environment", () => {
    expect(cheapRouterScript).toContain('PASEO_APP_NAME: "CheapRouter"');
    expect(cheapRouterScript).toContain('PASEO_CLOUD_NAME: "CheapRouter"');
    expect(cheapRouterScript).toContain('PASEO_LOGO_VARIANT: "cheaprouter"');
    expect(cheapRouterScript).toContain(
      'EXPO_PUBLIC_MANAGED_SERVICE_URL: "https://cheaprouter.org"',
    );
    expect(cheapRouterScript).toContain('PASEO_EXPO_ICON: "./assets/images/cheaprouter-icon.png"');
    expect(cheapRouterScript).toContain('PASEO_WEB_FAVICON: "./assets/images/cheaprouter-icon.png"');
    expect(cheapRouterScript).toContain('PASEO_DESKTOP_APP_ID: "org.cheaprouter.desktop"');
    expect(cheapRouterScript).toContain('PASEO_DESKTOP_ICON_PNG: "assets/cheaprouter-icon.png"');
    expect(cheapRouterScript).toContain('PASEO_DESKTOP_ICON_MAC: "assets/cheaprouter-icon.icns"');
    expect(cheapRouterScript).toContain('PASEO_DESKTOP_ICON_WIN: "assets/cheaprouter-icon.ico"');
    expect(cheapRouterScript).toContain('PASEO_DESKTOP_ICON_LINUX: "assets/cheaprouter"');
  });
});

describe("CyberAICoding brand script", () => {
  it("is exposed from root package scripts", () => {
    expect(rootPackageJson.scripts["with:cyberaicoding"]).toBe(
      "node scripts/run-with-cyberaicoding-brand.mjs",
    );
  });

  it("sets CyberAICoding app, icon, desktop, update, and cloud endpoint environment", () => {
    expect(cyberAiCodingScript).toContain('PASEO_APP_NAME: "CyberAICoding"');
    expect(cyberAiCodingScript).toContain('PASEO_CLOUD_NAME: "CyberAICoding Cloud"');
    expect(cyberAiCodingScript).toContain('PASEO_LOGO_VARIANT: "cybercode"');
    expect(cyberAiCodingScript).toContain(
      'EXPO_PUBLIC_MANAGED_SERVICE_URL: "https://ai-coding.cyberspirit.io"',
    );
    expect(cyberAiCodingScript).toContain('PASEO_EXPO_ICON: "./assets/images/cybercode-icon.png"');
    expect(cyberAiCodingScript).toContain('PASEO_WEB_FAVICON: "./assets/images/cybercode-favicon.png"');
    expect(cyberAiCodingScript).toContain('PASEO_DESKTOP_APP_ID: "com.cyberaicoding.desktop"');
    expect(cyberAiCodingScript).toContain('PASEO_DESKTOP_ICON_PNG: "assets/cybercode-icon.png"');
    expect(cyberAiCodingScript).toContain('PASEO_DESKTOP_ICON_MAC: "assets/cybercode-icon.icns"');
    expect(cyberAiCodingScript).toContain('PASEO_DESKTOP_ICON_WIN: "assets/cybercode-icon.ico"');
    expect(cyberAiCodingScript).toContain('PASEO_DESKTOP_ICON_LINUX: "assets/cybercode"');
    expect(cyberAiCodingScript).toContain(
      'PASEO_DESKTOP_UPDATE_URL: "https://minio.cyberspirit.io/"',
    );
  });
});
