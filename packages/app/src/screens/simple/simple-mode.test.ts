import { describe, expect, it } from "vitest";
import { isSimpleModeAgent, SIMPLE_EXPERIENCE_LABEL } from "./simple-mode";

describe("simple mode task filtering", () => {
  it("only accepts agents labeled for simple mode", () => {
    expect(isSimpleModeAgent({ labels: { experienceMode: SIMPLE_EXPERIENCE_LABEL } })).toBe(true);
    expect(isSimpleModeAgent({ labels: { experienceMode: "developer" } })).toBe(false);
    expect(isSimpleModeAgent({ labels: {} })).toBe(false);
  });
});
