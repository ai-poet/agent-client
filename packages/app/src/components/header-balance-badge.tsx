import { memo, useCallback, useMemo, useState } from "react";
import { Alert, Pressable, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { Wallet } from "lucide-react-native";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useAppSettings } from "@/hooks/use-settings";
import { useSub2APIAuth } from "@/hooks/use-sub2api-auth";
import { useSub2APIMe, useSub2APIUsageStats } from "@/hooks/use-sub2api-api";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { Sub2APIPayModal } from "@/screens/settings/sub2api-pay-modal";
import { formatUsd, getErrorMessage } from "@/screens/settings/managed-provider-settings-shared";

function getBalanceColor(
  balance: number | undefined,
  palette: { amber: Record<number, string>; red: Record<number, string> },
  muted: string,
  foreground: string,
): string {
  if (typeof balance !== "number") return muted;
  if (balance <= 0) return palette.red[500];
  if (balance < 1) return palette.amber[500];
  return foreground;
}

export const HeaderBalanceBadge = memo(function HeaderBalanceBadge() {
  const { theme } = useUnistyles();
  const { settings } = useAppSettings();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).authAlerts, [locale]);
  const sidebarText = useMemo(() => getSub2APIMessages(locale).sidebarUser, [locale]);
  const { isLoggedIn, auth, getAccessToken } = useSub2APIAuth();
  const meQuery = useSub2APIMe();
  const usageTodayQuery = useSub2APIUsageStats("today");

  const [isPayModalOpen, setIsPayModalOpen] = useState(false);
  const [payToken, setPayToken] = useState<string | null>(null);

  const isBuiltinLoggedIn = settings.accessMode === "builtin" && isLoggedIn;

  const handleOpenPayModal = useCallback(async () => {
    try {
      const token = await getAccessToken();
      if (!token) {
        Alert.alert(text.sessionExpired, text.loginAgain);
        return;
      }
      setPayToken(token);
      setIsPayModalOpen(true);
    } catch (error) {
      Alert.alert(text.unableOpenPayment, getErrorMessage(error));
    }
  }, [getAccessToken, text]);

  const handlePayCompleted = useCallback(() => {
    void meQuery.refetch();
    void usageTodayQuery.refetch();
  }, [meQuery, usageTodayQuery]);

  if (!isBuiltinLoggedIn) {
    return null;
  }

  const balance = meQuery.data?.balance;
  const balanceColor = getBalanceColor(
    balance,
    theme.colors.palette,
    theme.colors.foregroundMuted,
    theme.colors.foreground,
  );
  const todayCost = usageTodayQuery.data?.total_cost;
  const todayReqs = usageTodayQuery.data?.total_requests ?? 0;

  return (
    <>
      <Tooltip delayDuration={0} enabledOnDesktop enabledOnMobile={false}>
        <TooltipTrigger asChild>
          <Pressable
            onPress={() => void handleOpenPayModal()}
            style={({ hovered, pressed }) => [
              styles.badge,
              hovered && styles.badgeHovered,
              pressed && styles.badgePressed,
            ]}
            accessibilityRole="button"
            accessibilityLabel={`${sidebarText.balance} ${formatUsd(balance)}`}
            testID="header-balance-badge"
          >
            <Wallet size={14} color={balanceColor} />
            <Text style={[styles.balanceText, { color: balanceColor }]} numberOfLines={1}>
              {meQuery.isLoading ? "..." : formatUsd(balance)}
            </Text>
          </Pressable>
        </TooltipTrigger>
        <TooltipContent side="bottom" align="center" offset={8}>
          <Text style={styles.tooltipText}>
            {sidebarText.todayUsage(formatUsd(todayCost), todayReqs)}
          </Text>
        </TooltipContent>
      </Tooltip>

      <Sub2APIPayModal
        visible={isPayModalOpen}
        endpoint={auth?.endpoint ?? ""}
        accessToken={payToken}
        onClose={() => setIsPayModalOpen(false)}
        onCompleted={handlePayCompleted}
      />
    </>
  );
});

const styles = StyleSheet.create((theme) => ({
  badge: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[1],
    height: 28,
    paddingHorizontal: theme.spacing[2],
    borderRadius: theme.borderRadius["2xl"],
    backgroundColor: "transparent",
  },
  badgeHovered: {
    backgroundColor: theme.colors.surface2,
  },
  badgePressed: {
    backgroundColor: theme.colors.surface0,
  },
  balanceText: {
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
  },
  tooltipText: {
    color: theme.colors.popoverForeground,
    fontSize: theme.fontSize.sm,
  },
}));
