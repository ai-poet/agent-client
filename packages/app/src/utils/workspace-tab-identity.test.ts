import { describe, expect, it } from "vitest";
import {
  buildDeterministicWorkspaceTabId,
  normalizeWorkspaceTabTarget,
  workspaceTabTargetsEqual,
} from "@/utils/workspace-tab-identity";

describe("workspace tab identity", () => {
  it("normalizes preview targets by script name", () => {
    expect(normalizeWorkspaceTabTarget({ kind: "preview", scriptName: "  web  " })).toEqual({
      kind: "preview",
      scriptName: "web",
    });
    expect(normalizeWorkspaceTabTarget({ kind: "preview", scriptName: "   " })).toBeNull();
  });

  it("compares preview targets by script name", () => {
    expect(
      workspaceTabTargetsEqual(
        { kind: "preview", scriptName: "web" },
        { kind: "preview", scriptName: "web" },
      ),
    ).toBe(true);
    expect(
      workspaceTabTargetsEqual(
        { kind: "preview", scriptName: "web" },
        { kind: "preview", scriptName: "api" },
      ),
    ).toBe(false);
  });

  it("builds deterministic preview tab ids", () => {
    expect(buildDeterministicWorkspaceTabId({ kind: "preview", scriptName: "web" })).toBe(
      "preview_web",
    );
  });
});
