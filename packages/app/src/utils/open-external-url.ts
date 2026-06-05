import { getDesktopHost } from "@/desktop/host";
import { isWeb } from "@/constants/platform";

export async function openExternalUrl(url: string): Promise<void> {
  if (isWeb) {
    const opener = getDesktopHost()?.opener?.openUrl;
    if (typeof opener === "function") {
      await opener(url);
      return;
    }

    window.open(url, "_blank", "noopener,noreferrer");
    return;
  }

  const Linking = await import("expo-linking");
  await Linking.openURL(url);
}
