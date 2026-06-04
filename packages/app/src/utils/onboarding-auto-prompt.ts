export function isOnboardingAutoPromptRoute(input: {
  pathname: string | null | undefined;
  chromeEnabled: boolean;
}): boolean {
  if (!input.chromeEnabled) {
    return false;
  }

  const pathname = input.pathname?.trim() ?? "";
  if (!pathname) {
    return false;
  }

  const hostMatch = pathname.match(/^\/h\/[^/]+(?:\/([^/?#]+))?(?:[/?#]|$)/);
  if (!hostMatch) {
    return false;
  }

  const section = hostMatch[1] ?? "";
  if (!section) {
    return true;
  }

  if (
    section === "workspace" ||
    section === "open-project" ||
    section === "simple" ||
    section === "settings"
  ) {
    return false;
  }

  return true;
}
