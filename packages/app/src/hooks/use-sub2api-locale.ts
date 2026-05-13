import { useMemo } from "react";
import { useAppSettings } from "@/hooks/use-settings";
import { resolveSub2APILocaleFromPreference, type Sub2APILocale } from "@/i18n/sub2api";

export function useSub2APILocale(): Sub2APILocale {
  const { settings } = useAppSettings();
  return useMemo(() => resolveSub2APILocaleFromPreference(settings.language), [settings.language]);
}
