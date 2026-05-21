import { describe, expect, it } from "vitest";
import {
  appMessages,
  filterSub2APIPaymentTypesByLocale,
  getAppMessages,
  getSub2APIMessages,
  getSub2APIPaymentLabel,
  normalizeSub2APILanguagePreference,
  normalizeSub2APILocale,
  resolveSub2APILocaleFromPreference,
  resolveSub2APILocale,
} from "./sub2api";

function collectMessageShape(value: unknown, path = ""): string[] {
  if (typeof value === "function") {
    return [`${path}:function`];
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return [`${path}:${typeof value}`];
  }
  return Object.keys(value)
    .sort()
    .flatMap((key) =>
      collectMessageShape(
        (value as Record<string, unknown>)[key],
        path.length > 0 ? `${path}.${key}` : key,
      ),
    );
}

describe("sub2api i18n helpers", () => {
  it("normalizes supported Chinese and English locales", () => {
    expect(normalizeSub2APILocale("zh-CN")).toBe("zh");
    expect(normalizeSub2APILocale("zh-Hant")).toBe("zh");
    expect(normalizeSub2APILocale("en-US")).toBe("en");
    expect(normalizeSub2APILocale("fr-FR")).toBe("en");
  });

  it("resolves explicit locale before system locale", () => {
    expect(resolveSub2APILocale({ explicitLocale: "zh-CN", intlLocale: "en-US" })).toBe("zh");
    expect(resolveSub2APILocale({ intlLocale: "en-US", navigatorLanguages: ["zh-CN"] })).toBe("en");
  });

  it("resolves app language preference with system fallback", () => {
    expect(normalizeSub2APILanguagePreference("zh-CN")).toBe("zh");
    expect(normalizeSub2APILanguagePreference("en-US")).toBe("en");
    expect(normalizeSub2APILanguagePreference("auto")).toBe("auto");
    expect(normalizeSub2APILanguagePreference("fr-FR")).toBe("auto");
    expect(resolveSub2APILocaleFromPreference("auto", { intlLocale: "zh-CN" })).toBe("zh");
    expect(resolveSub2APILocaleFromPreference("en", { intlLocale: "zh-CN" })).toBe("en");
  });

  it("localizes payment labels", () => {
    expect(getSub2APIPaymentLabel("alipay", "zh")).toBe("支付宝");
    expect(getSub2APIPaymentLabel("wxpay_direct", "zh")).toBe("微信支付");
    expect(getSub2APIPaymentLabel("alipay", "en")).toBe("Alipay");
    expect(getSub2APIPaymentLabel("wxpay_direct", "en")).toBe("WeChat Pay");
    expect(getSub2APIPaymentLabel("usdt.polygon", "en")).toBe("USDT (Polygon)");
  });

  it("filters payment methods by locale like the Sub2API pay center", () => {
    const allTypes = ["alipay", "wxpay_direct", "bank", "stripe", "usdt.plasma"];

    expect(filterSub2APIPaymentTypesByLocale(allTypes, "zh")).toEqual(["alipay", "wxpay_direct"]);
    expect(filterSub2APIPaymentTypesByLocale(allTypes, "en")).toEqual(["bank"]);
  });

  it("exposes Chinese and English UI copy", () => {
    expect(getSub2APIMessages("zh").pay.addBalance).toBe("充值余额");
    expect(getSub2APIMessages("en").pay.addBalance).toBe("Add balance");
    expect(getSub2APIMessages("zh").modelCatalog.title).toBe("模型广场");
    expect(getSub2APIMessages("en").modelCatalog.title).toBe("Model catalog");
    expect(getSub2APIMessages("zh").settings.sections.general).toBe("通用");
    expect(getSub2APIMessages("en").settings.sections.general).toBe("General");
    expect(getSub2APIMessages("zh").cloudPanel.signInBody("Paseo Cloud", "Paseo")).toContain(
      "使用我们自托管的AI模型服务",
    );
    expect(getSub2APIMessages("en").cloudPanel.signInBody("Paseo Cloud", "Paseo")).toContain(
      "sub-managed AI model service",
    );
    expect(getSub2APIMessages("zh").sidebar.projects).toBe("项目");
    expect(getSub2APIMessages("en").sidebar.projects).toBe("Projects");
    expect(getSub2APIMessages("zh").sidebar.chat).toBe("聊天");
    expect(getSub2APIMessages("en").sidebar.chat).toBe("Chat");
    expect(getSub2APIMessages("zh").paseoCloudApiKeys.title).toBe("API 密钥（高级）");
    expect(getSub2APIMessages("en").paseoCloudApiKeys.title).toBe("API Keys (advanced)");
    expect(getSub2APIMessages("zh").paseoCloudUsage.title).toBe("用量");
    expect(getSub2APIMessages("en").paseoCloudUsage.title).toBe("Usage");
    expect(getSub2APIMessages("zh").setupCheck.title).toBe("环境检查");
    expect(getSub2APIMessages("en").setupCheck.title).toBe("Environment Check");
    expect(getSub2APIMessages("zh").loginScreen.primaryLogin).toBe("登录");
    expect(getSub2APIMessages("en").loginScreen.primaryLogin).toBe("Sign in");
    expect(getSub2APIMessages("zh").modeSelect.signInMeta).toBe("登录 · 推荐");
    expect(getSub2APIMessages("en").modeSelect.signInMeta).toBe("Sign in · Recommended");
    expect(getSub2APIMessages("zh").authAlerts.loginFailed).toBe("登录失败");
    expect(getSub2APIMessages("en").authAlerts.loginFailed).toBe("Login failed");
  });

  it("keeps Chinese and English message trees structurally aligned", () => {
    expect(collectMessageShape(appMessages.zh)).toEqual(collectMessageShape(appMessages.en));
  });

  it("exposes app workflow aliases and localized workflow copy", () => {
    expect(getAppMessages("zh").openProject.addProject).toBe("添加项目");
    expect(getAppMessages("en").openProject.addProject).toBe("Add a project");
    expect(getAppMessages("zh").newWorkspace.title).toBe("新建工作区");
    expect(getAppMessages("en").newWorkspace.title).toBe("New workspace");
    expect(getAppMessages("zh").workspace.tabLabels.newAgent).toBe("新建代理");
    expect(getAppMessages("en").workspace.tabLabels.newAgent).toBe("New Agent");
    expect(getAppMessages("zh").gitDiff.noUncommittedChanges).toBe("没有未提交更改");
    expect(getAppMessages("en").gitDiff.noUncommittedChanges).toBe("No uncommitted changes");
    expect(getAppMessages("zh").composer.send).toBe("发送");
    expect(getAppMessages("en").composer.send).toBe("Send");
    expect(getAppMessages("zh").modelSelector.favorites).toBe("收藏");
    expect(getAppMessages("en").modelSelector.favorites).toBe("Favorites");
    expect(getAppMessages("zh").settings.addHost.title).toBe("添加连接");
    expect(getAppMessages("en").settings.addHost.title).toBe("Add connection");
    expect(getAppMessages("zh").settings.host.sections.dangerZone).toBe("危险区");
    expect(getAppMessages("en").settings.host.sections.dangerZone).toBe("Danger zone");
  });
});
