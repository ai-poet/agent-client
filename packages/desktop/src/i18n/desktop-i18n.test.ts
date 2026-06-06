import { afterEach, describe, expect, it } from "vitest";
import {
  __setDesktopLocaleForTests,
  getDesktopMessage,
  normalizeDesktopLocale,
} from "./desktop-i18n";

describe("desktop-i18n", () => {
  afterEach(() => {
    __setDesktopLocaleForTests(null);
  });

  it("normalizes Chinese locales", () => {
    expect(normalizeDesktopLocale("zh-CN")).toBe("zh");
    expect(normalizeDesktopLocale("zh-Hant")).toBe("zh");
    expect(normalizeDesktopLocale("en-US")).toBe("en");
    expect(normalizeDesktopLocale(null)).toBe("en");
  });

  it("formats localized desktop errors", () => {
    __setDesktopLocaleForTests("zh");

    expect(getDesktopMessage("provider.notFound", { id: "missing" })).toBe("未找到提供商：missing");
    expect(getDesktopMessage("opener.unsupportedExternalUrl")).toBe("不支持此外部 URL。");
    expect(
      getDesktopMessage("cli.installFailed", {
        message: "npm failed",
        missingText: "Codex",
      }),
    ).toBe("安装失败：npm failed 缺少：Codex");
  });
});
