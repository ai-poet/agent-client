import { useCallback } from "react";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { THEME_TO_UNISTYLES, type ThemeName } from "@/styles/theme";

export const APP_SETTINGS_KEY = "@paseo:app-settings";
const LEGACY_SETTINGS_KEY = "@paseo:settings";
const APP_SETTINGS_QUERY_KEY = ["app-settings"];

export type SendBehavior = "interrupt" | "queue";
export type ReleaseChannel = "stable" | "beta";
export type AccessMode = "builtin" | "byok";
export type AppLanguage = "auto" | "zh" | "en";

const VALID_THEMES = new Set<string>([...Object.keys(THEME_TO_UNISTYLES), "auto"]);
const VALID_RELEASE_CHANNELS = new Set<string>(["stable", "beta"]);
const VALID_ACCESS_MODES = new Set<string>(["builtin", "byok"]);
const VALID_APP_LANGUAGES = new Set<string>(["auto", "zh", "en"]);

export interface AppSettings {
  theme: ThemeName | "auto";
  manageBuiltInDaemon: boolean;
  sendBehavior: SendBehavior;
  releaseChannel: ReleaseChannel;
  accessMode: AccessMode | null;
  setupCheckCompleted: boolean;
  onboardingGuideCompleted: boolean;
  language: AppLanguage;
}

export const DEFAULT_APP_SETTINGS: AppSettings = {
  theme: "auto",
  manageBuiltInDaemon: true,
  sendBehavior: "interrupt",
  releaseChannel: "stable",
  accessMode: null,
  setupCheckCompleted: false,
  onboardingGuideCompleted: false,
  language: "auto",
};

export interface UseAppSettingsReturn {
  settings: AppSettings;
  isLoading: boolean;
  error: unknown | null;
  updateSettings: (updates: Partial<AppSettings>) => Promise<void>;
  resetSettings: () => Promise<void>;
}

export function useAppSettings(): UseAppSettingsReturn {
  const queryClient = useQueryClient();
  const { data, isPending, error } = useQuery({
    queryKey: APP_SETTINGS_QUERY_KEY,
    queryFn: loadSettingsFromStorage,
    staleTime: Infinity,
    gcTime: Infinity,
  });

  const updateSettings = useCallback(
    async (updates: Partial<AppSettings>) => {
      try {
        const prev =
          queryClient.getQueryData<AppSettings>(APP_SETTINGS_QUERY_KEY) ?? DEFAULT_APP_SETTINGS;
        const next = { ...prev, ...updates };
        queryClient.setQueryData<AppSettings>(APP_SETTINGS_QUERY_KEY, next);
        await AsyncStorage.setItem(APP_SETTINGS_KEY, JSON.stringify(next));
      } catch (err) {
        console.error("[AppSettings] Failed to save settings:", err);
        throw err;
      }
    },
    [queryClient],
  );

  const resetSettings = useCallback(async () => {
    try {
      const next = { ...DEFAULT_APP_SETTINGS };
      queryClient.setQueryData<AppSettings>(APP_SETTINGS_QUERY_KEY, next);
      await AsyncStorage.setItem(APP_SETTINGS_KEY, JSON.stringify(next));
    } catch (err) {
      console.error("[AppSettings] Failed to reset settings:", err);
      throw err;
    }
  }, [queryClient]);

  return {
    settings: data ?? DEFAULT_APP_SETTINGS,
    isLoading: isPending,
    error: error ?? null,
    updateSettings,
    resetSettings,
  };
}

export async function loadSettingsFromStorage(): Promise<AppSettings> {
  try {
    const stored = await AsyncStorage.getItem(APP_SETTINGS_KEY);
    if (stored) {
      const parsed = JSON.parse(stored) as Partial<AppSettings>;
      if (parsed.theme && !VALID_THEMES.has(parsed.theme)) {
        parsed.theme = DEFAULT_APP_SETTINGS.theme;
      }
      if (parsed.releaseChannel && !VALID_RELEASE_CHANNELS.has(parsed.releaseChannel)) {
        parsed.releaseChannel = DEFAULT_APP_SETTINGS.releaseChannel;
      }
      if (
        parsed.accessMode !== null &&
        parsed.accessMode !== undefined &&
        !VALID_ACCESS_MODES.has(parsed.accessMode)
      ) {
        parsed.accessMode = DEFAULT_APP_SETTINGS.accessMode;
      }
      if (parsed.language && !VALID_APP_LANGUAGES.has(parsed.language)) {
        parsed.language = DEFAULT_APP_SETTINGS.language;
      }
      if (
        parsed.onboardingGuideCompleted !== undefined &&
        typeof parsed.onboardingGuideCompleted !== "boolean"
      ) {
        parsed.onboardingGuideCompleted = DEFAULT_APP_SETTINGS.onboardingGuideCompleted;
      }
      return { ...DEFAULT_APP_SETTINGS, ...parsed };
    }

    const legacyStored = await AsyncStorage.getItem(LEGACY_SETTINGS_KEY);
    if (legacyStored) {
      const legacyParsed = JSON.parse(legacyStored) as Record<string, unknown>;
      const next = {
        ...DEFAULT_APP_SETTINGS,
        ...pickAppSettingsFromLegacy(legacyParsed),
      } satisfies AppSettings;
      await AsyncStorage.setItem(APP_SETTINGS_KEY, JSON.stringify(next));
      return next;
    }

    await AsyncStorage.setItem(APP_SETTINGS_KEY, JSON.stringify(DEFAULT_APP_SETTINGS));
    return DEFAULT_APP_SETTINGS;
  } catch (error) {
    console.error("[AppSettings] Failed to load settings:", error);
    throw error;
  }
}

function pickAppSettingsFromLegacy(legacy: Record<string, unknown>): Partial<AppSettings> {
  const result: Partial<AppSettings> = {};
  if (legacy.theme === "dark" || legacy.theme === "light" || legacy.theme === "auto") {
    result.theme = legacy.theme;
  }
  if (typeof legacy.manageBuiltInDaemon === "boolean") {
    result.manageBuiltInDaemon = legacy.manageBuiltInDaemon;
  }
  if (legacy.releaseChannel === "stable" || legacy.releaseChannel === "beta") {
    result.releaseChannel = legacy.releaseChannel;
  }
  if (legacy.language === "auto" || legacy.language === "zh" || legacy.language === "en") {
    result.language = legacy.language;
  }
  return result;
}

export const useSettings = useAppSettings;
