import { View, Text } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { RotateCw } from "lucide-react-native";
import { Button } from "@/components/ui/button";
import { DesktopPermissionRow } from "@/desktop/components/desktop-permission-row";
import { useDesktopPermissions } from "@/desktop/permissions/use-desktop-permissions";
import { settingsStyles } from "@/styles/settings";
import { SettingsSection } from "@/screens/settings/settings-section";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { useMemo } from "react";

export function DesktopPermissionsSection() {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.permissions, [locale]);
  const {
    isDesktopApp,
    snapshot,
    isRefreshing,
    requestingPermission,
    isSendingTestNotification,
    testNotificationError,
    refreshPermissions,
    requestPermission,
    sendTestNotification,
  } = useDesktopPermissions();

  if (!isDesktopApp) {
    return null;
  }

  const isBusy = isRefreshing || requestingPermission !== null;
  const notificationsGranted = snapshot?.notifications.state === "granted";

  const refreshButton = (
    <Button
      variant="ghost"
      size="sm"
      leftIcon={<RotateCw size={theme.iconSize.md} color={theme.colors.foregroundMuted} />}
      onPress={() => {
        void refreshPermissions();
      }}
      disabled={isBusy}
      accessibilityLabel={text.refreshDesktopPermissions}
    >
      {isRefreshing ? text.refreshing : text.refresh}
    </Button>
  );

  return (
    <SettingsSection title={text.title} trailing={refreshButton}>
      <View style={settingsStyles.card}>
        <DesktopPermissionRow
          title={text.notifications}
          status={snapshot?.notifications ?? null}
          isRequesting={requestingPermission === "notifications"}
          onRequest={() => {
            void requestPermission("notifications");
          }}
          extraActionLabel={text.test}
          isExtraActionBusy={isSendingTestNotification}
          isExtraActionDisabled={!notificationsGranted || isBusy}
          onExtraAction={() => {
            void sendTestNotification();
          }}
        />
        {testNotificationError ? (
          <Text style={[styles.errorText, { color: theme.colors.destructive }]}>
            {testNotificationError}
          </Text>
        ) : null}
        <DesktopPermissionRow
          title={text.microphone}
          showBorder
          status={snapshot?.microphone ?? null}
          isRequesting={requestingPermission === "microphone"}
          onRequest={() => {
            void requestPermission("microphone");
          }}
        />
      </View>
    </SettingsSection>
  );
}

const styles = StyleSheet.create((theme) => ({
  errorText: {
    fontSize: theme.fontSize.xs,
    paddingHorizontal: theme.spacing[4],
    paddingBottom: theme.spacing[2],
  },
}));
