import { useCallback, useMemo } from "react";
import { Text, View } from "react-native";
import { useRouter } from "expo-router";
import { useUnistyles } from "react-native-unistyles";
import { getIsElectron } from "@/constants/platform";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { useAppSettings } from "@/hooks/use-settings";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { settingsStyles } from "@/styles/settings";
import { SettingsSection } from "@/screens/settings/settings-section";
import { Button } from "@/components/ui/button";
import { CLOUD_NAME } from "@/config/branding";

export function AccessModeSection() {
  const router = useRouter();
  const { theme } = useUnistyles();
  const { settings, updateSettings } = useAppSettings();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.accessMode, [locale]);
  const isElectron = getIsElectron();

  const modeLabel =
    settings.accessMode === "builtin"
      ? CLOUD_NAME
      : settings.accessMode === "byok"
        ? "BYOK"
        : text.notSelected;

  const handleSwitchMode = useCallback(async () => {
    await updateSettings({ accessMode: null, setupCheckCompleted: false });
    router.replace("/mode-select");
  }, [router, updateSettings]);

  if (!isElectron) {
    return null;
  }

  return (
    <SettingsSection title={text.title}>
      <View style={[settingsStyles.card, { gap: theme.spacing[3], padding: theme.spacing[4] }]}>
        <Text style={{ color: theme.colors.foregroundMuted, fontSize: theme.fontSize.sm }}>
          {text.current(modeLabel)}
        </Text>
        <Text style={{ color: theme.colors.foregroundMuted, fontSize: theme.fontSize.xs }}>
          {text.hint(CLOUD_NAME)}
        </Text>
        <Button variant="secondary" size="sm" onPress={() => void handleSwitchMode()}>
          {text.change}
        </Button>
      </View>
    </SettingsSection>
  );
}
