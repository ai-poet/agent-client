import { describe, expect, it } from "vitest";
import {
  localizeAgentMode,
  localizeAgentModeDescription,
  localizeAgentModeLabel,
} from "./agent-mode-localization";

describe("agent mode localization", () => {
  it("localizes Claude permission labels and descriptions in Chinese", () => {
    expect(localizeAgentModeLabel({ id: "default", label: "Always Ask" }, "zh")).toBe("始终询问");
    expect(localizeAgentModeLabel({ id: "bypassPermissions", label: "Bypass" }, "zh")).toBe(
      "绕过权限",
    );
    expect(
      localizeAgentModeDescription(
        { id: "bypassPermissions", label: "Bypass", description: "No permissions" },
        "zh",
      ),
    ).toBe("跳过所有权限提示（请谨慎使用）");
  });

  it("uses the mode id before provider-supplied English labels", () => {
    expect(localizeAgentModeLabel({ id: "full-access", label: "ByPass" }, "zh")).toBe("完全访问");
    expect(localizeAgentModeLabel({ id: "always-ask", label: "Default" }, "zh")).toBe("始终询问");
  });

  it("leaves unknown custom modes unchanged", () => {
    const mode = localizeAgentMode(
      {
        id: "custom-review",
        label: "Review",
        description: "Provider supplied custom mode",
      },
      "zh",
    );

    expect(mode).toEqual({
      id: "custom-review",
      label: "Review",
      description: "Provider supplied custom mode",
    });
  });
});
