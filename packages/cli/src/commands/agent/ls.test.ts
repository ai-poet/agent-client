import { describe, expect, it } from "vitest";
import { buildAgentLsFetchOptions } from "./ls.js";

describe("buildAgentLsFetchOptions", () => {
  it("fetches active agents by default", () => {
    expect(buildAgentLsFetchOptions({})).toEqual({});
  });

  it("keeps label and thinking filters", () => {
    expect(
      buildAgentLsFetchOptions({
        label: ["surface=workspace"],
        thinking: " medium ",
      }),
    ).toEqual({
      filter: {
        labels: { surface: "workspace" },
        thinkingOptionId: "medium",
      },
    });
  });

  it("uses the unscoped archived query for -a", () => {
    expect(buildAgentLsFetchOptions({ all: true })).toEqual({
      filter: {
        includeArchived: true,
      },
    });
  });
});
