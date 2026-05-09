import { createRequire } from "node:module";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const require = createRequire(import.meta.url);
const workspaceRoot = path.resolve(__dirname, "../../..");
const desktopPackageRoot = path.resolve(__dirname, "..");
const builderConfigPath = path.join(desktopPackageRoot, "electron-builder.config.cjs");
const brandingConfigPath = path.join(desktopPackageRoot, "branding.cjs");
const rootPackageJsonPath = path.join(workspaceRoot, "package.json");
const desktopPackageJsonPath = path.join(desktopPackageRoot, "package.json");

const BRAND_ENV_KEYS = [
  "PASEO_APP_NAME",
  "PASEO_DESKTOP_APP_ID",
  "PASEO_DESKTOP_ICON_PNG",
  "PASEO_DESKTOP_ICON_MAC",
  "PASEO_DESKTOP_ICON_WIN",
  "PASEO_DESKTOP_ICON_LINUX",
];

function loadBuilderConfig(env: NodeJS.ProcessEnv = {}) {
  const previousEnv = new Map<string, string | undefined>();
  for (const key of BRAND_ENV_KEYS) {
    previousEnv.set(key, process.env[key]);
    delete process.env[key];
  }

  Object.assign(process.env, env);
  delete require.cache[require.resolve(builderConfigPath)];
  delete require.cache[require.resolve(brandingConfigPath)];

  try {
    return require(builderConfigPath) as {
      appId: string;
      productName: string;
      executableName: string;
      win: {
        artifactName?: string;
        icon: string;
      };
      mac: {
        artifactName?: string;
        icon: string;
      };
    };
  } finally {
    delete require.cache[require.resolve(builderConfigPath)];
    delete require.cache[require.resolve(brandingConfigPath)];
    for (const [key, value] of previousEnv) {
      if (value === undefined) {
        delete process.env[key];
      } else {
        process.env[key] = value;
      }
    }
  }
}

describe("desktop packaging", () => {
  afterEach(() => {
    delete require.cache[require.resolve(builderConfigPath)];
    delete require.cache[require.resolve(brandingConfigPath)];
  });

  it("keeps the default Paseo package identity", () => {
    const config = loadBuilderConfig();

    expect(config.appId).toBe("sh.paseo.desktop");
    expect(config.productName).toBe("Paseo");
    expect(config.executableName).toBe("Paseo");
    expect(config.mac.artifactName).toBe("Paseo-${version}-${arch}.${ext}");
    expect(config.win.artifactName).toBe("Paseo-${version}-${arch}.${ext}");
  });

  it("uses CyberAICoding package identity and icons from env", () => {
    const config = loadBuilderConfig({
      PASEO_APP_NAME: "CyberAICoding",
      PASEO_DESKTOP_APP_ID: "com.cyberaicoding.desktop",
      PASEO_DESKTOP_ICON_MAC: "assets/cybercode-icon.icns",
      PASEO_DESKTOP_ICON_WIN: "assets/cybercode-icon.ico",
    });

    expect(config.appId).toBe("com.cyberaicoding.desktop");
    expect(config.productName).toBe("CyberAICoding");
    expect(config.executableName).toBe("CyberAICoding");
    expect(config.mac.icon).toBe("assets/cybercode-icon.icns");
    expect(config.win.icon).toBe("assets/cybercode-icon.ico");
    expect(config.mac.artifactName).toBe("CyberAICoding-${version}-${arch}.${ext}");
    expect(config.win.artifactName).toBe("CyberAICoding-${version}-${arch}.${ext}");
  });

  it("forwards root build:desktop flags to the desktop workspace builder", () => {
    const packageJson = require(rootPackageJsonPath) as {
      scripts: Record<string, string>;
    };

    expect(packageJson.scripts["build:desktop"]).toContain(
      "npm run build --workspace=@getpaseo/desktop --",
    );
  });

  it("cleans the desktop main-process dist before compiling", () => {
    const packageJson = require(desktopPackageJsonPath) as {
      scripts: Record<string, string>;
    };

    expect(packageJson.scripts["build:main"]).toContain("node scripts/clean-dist.mjs");
    expect(packageJson.scripts["build:main"]).toContain("tsc -p tsconfig.json");
    expect(packageJson.scripts["build:main"]).toContain("--incremental false");
  });
});
