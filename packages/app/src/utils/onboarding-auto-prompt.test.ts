import { describe, expect, it } from "vitest";
import { isOnboardingAutoPromptRoute } from "./onboarding-auto-prompt";

describe("isOnboardingAutoPromptRoute", () => {
  it("allows host main chrome routes", () => {
    expect(isOnboardingAutoPromptRoute({ pathname: "/h/local", chromeEnabled: true })).toBe(true);
    expect(
      isOnboardingAutoPromptRoute({ pathname: "/h/local/sessions", chromeEnabled: true }),
    ).toBe(true);
    expect(isOnboardingAutoPromptRoute({ pathname: "/h/local/skills", chromeEnabled: true })).toBe(
      true,
    );
  });

  it("blocks routes that should not auto-open onboarding", () => {
    expect(
      isOnboardingAutoPromptRoute({ pathname: "/h/local/open-project", chromeEnabled: true }),
    ).toBe(false);
    expect(
      isOnboardingAutoPromptRoute({
        pathname: "/h/local/simple/agent/123",
        chromeEnabled: true,
      }),
    ).toBe(false);
    expect(
      isOnboardingAutoPromptRoute({
        pathname: "/h/local/workspace/ws1",
        chromeEnabled: true,
      }),
    ).toBe(false);
    expect(
      isOnboardingAutoPromptRoute({ pathname: "/h/local/settings", chromeEnabled: true }),
    ).toBe(false);
    expect(isOnboardingAutoPromptRoute({ pathname: "/settings", chromeEnabled: true })).toBe(false);
    expect(isOnboardingAutoPromptRoute({ pathname: "/login", chromeEnabled: true })).toBe(false);
    expect(isOnboardingAutoPromptRoute({ pathname: "/welcome", chromeEnabled: true })).toBe(false);
  });

  it("requires chrome to be enabled", () => {
    expect(isOnboardingAutoPromptRoute({ pathname: "/h/local", chromeEnabled: false })).toBe(false);
  });
});
