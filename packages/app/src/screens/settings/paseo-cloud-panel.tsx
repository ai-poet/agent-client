import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Alert, Pressable, Text, View } from "react-native";
import { useRouter } from "expo-router";
import { useUnistyles } from "react-native-unistyles";
import { Cloud } from "lucide-react-native";
import { APP_NAME, CLOUD_NAME } from "@/config/branding";
import { useIsCompactFormFactor } from "@/constants/layout";
import { getManagedServiceUrlFromEnv } from "@/config/managed-service-env";
import { useSub2APIAuth } from "@/hooks/use-sub2api-auth";
import { useSub2APILoginFlow } from "@/hooks/use-sub2api-login-flow";
import { useAppSettings } from "@/hooks/use-settings";
import { useSub2APIMe, useSub2APIKeys, useSub2APIUsageStats } from "@/hooks/use-sub2api-api";
import type { Sub2APIKey } from "@/lib/sub2api-client";
import { SettingsSection } from "@/screens/settings/settings-section";
import { useDesktopProvidersStore } from "@/screens/settings/desktop-providers-context";
import type { DesktopProviderPayload } from "@/screens/settings/sub2api-provider-types";
import { managedProviderSettingsStyles as styles } from "@/screens/settings/managed-provider-settings-styles";
import { Sub2APIPayModal } from "@/screens/settings/sub2api-pay-modal";
import { Sub2APIModelsSection } from "@/screens/settings/sub2api-models-section";
import { PaseoCloudApiKeysSection } from "@/screens/settings/paseo-cloud-api-keys-section";
import { PaseoCloudRoutingSection } from "@/screens/settings/paseo-cloud-routing-section";
import { PaseoCloudReferralSection } from "@/screens/settings/paseo-cloud-referral-section";
import { PaseoCloudUsageSection } from "@/screens/settings/paseo-cloud-usage-section";
import { PaseoCloudModelStatusSection } from "@/screens/settings/paseo-cloud-model-status-section";
import { settingsStyles } from "@/styles/settings";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { formatUsd, getErrorMessage, maskApiKey } from "./managed-provider-settings-shared";

type PaseoCloudSection =
  | "overview"
  | "keys"
  | "routing"
  | "catalog"
  | "usage"
  | "status"
  | "referral";

const SECTION_OPTIONS: {
  id: PaseoCloudSection;
  testID: string;
}[] = [
  { id: "overview", testID: "paseo-cloud-section-overview" },
  { id: "keys", testID: "paseo-cloud-section-keys" },
  { id: "routing", testID: "paseo-cloud-section-routing" },
  {
    id: "catalog",
    testID: "paseo-cloud-section-catalog",
  },
  { id: "usage", testID: "paseo-cloud-section-usage" },
  { id: "status", testID: "paseo-cloud-section-status" },
  { id: "referral", testID: "paseo-cloud-section-referral" },
];

type CloudPanelText = ReturnType<typeof getSub2APIMessages>["cloudPanel"];

type RouteUsageCard = {
  scopeLabel: string;
  provider: DesktopProviderPayload | null;
  cloudKey: Sub2APIKey | null;
  todayCost: number | null | undefined;
  todayRequests: number | null | undefined;
};

function getSectionLabel(section: PaseoCloudSection, text: CloudPanelText): string {
  return text.sections[section];
}

function routeActionId(scopeLabel: string): string {
  return scopeLabel
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
}

function getLocalTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "";
  } catch {
    return "";
  }
}

function findCloudKeyByApiKey(
  keys: Sub2APIKey[],
  apiKey: string | null | undefined,
): Sub2APIKey | null {
  const trimmed = apiKey?.trim();
  if (!trimmed) {
    return null;
  }
  return keys.find((key) => key.key.trim() === trimmed) ?? null;
}

function RouteUsageCardBlock({
  card,
  text,
  onOpenKeys,
  onOpenRouting,
}: {
  card: RouteUsageCard;
  text: CloudPanelText;
  onOpenKeys: () => void;
  onOpenRouting: () => void;
}) {
  const isCloudBacked = Boolean(card.cloudKey);
  const quotaRatio =
    card.cloudKey && card.cloudKey.quota > 0 ? card.cloudKey.quota_used / card.cloudKey.quota : 0;
  const actionId = routeActionId(card.scopeLabel);

  if (!card.provider) {
    return (
      <View style={[styles.routeSummaryCard, styles.routeSummaryCardMuted]}>
        <View style={styles.routeSummaryHeader}>
          <Text style={styles.routeSummaryTitle}>{card.scopeLabel}</Text>
          <Text style={[styles.routeSummaryBadge, styles.infoBadgeNeutral]}>
            {text.notConfigured}
          </Text>
        </View>
        <Text style={styles.routeSummaryProviderHint}>{text.noActiveRoute(card.scopeLabel)}</Text>
      </View>
    );
  }

  return (
    <View style={styles.routeSummaryCard}>
      <View style={styles.routeSummaryHeader}>
        <Text style={styles.routeSummaryTitle}>{card.scopeLabel}</Text>
        <Text
          style={[
            styles.routeSummaryBadge,
            isCloudBacked ? styles.routeSummaryBadgeCloud : styles.routeSummaryBadgeCustom,
          ]}
        >
          {isCloudBacked ? CLOUD_NAME : text.customRoute}
        </Text>
      </View>
      <Text style={styles.routeSummaryProviderName}>{card.provider.name}</Text>
      <Text style={styles.routeSummaryProviderHint}>{card.provider.endpoint}</Text>
      <Text style={styles.routeSummaryProviderHint}>
        {text.key} {maskApiKey(card.provider.apiKey)}
      </Text>
      {card.cloudKey ? (
        <>
          <Text style={styles.routeSummaryProviderHint}>
            {text.group} {card.cloudKey.group?.name ?? card.cloudKey.group_id ?? text.none}
          </Text>
          <View style={styles.routeSummaryMetricsRow}>
            <View style={styles.routeMetricCard}>
              <Text style={styles.routeMetricLabel}>{text.today}</Text>
              <Text style={styles.routeMetricValue}>{formatUsd(card.todayCost)}</Text>
              <Text style={styles.routeMetricSubvalue}>
                {card.todayRequests ?? 0} {text.requests}
              </Text>
            </View>
            <View style={styles.routeMetricCard}>
              <Text style={styles.routeMetricLabel}>{text.totalSpend}</Text>
              <Text style={styles.routeMetricValue}>{formatUsd(card.cloudKey.quota_used)}</Text>
              <Text style={styles.routeMetricSubvalue}>
                {card.cloudKey.quota > 0
                  ? `${formatUsd(card.cloudKey.quota)} ${text.quota}`
                  : text.unlimitedQuota}
              </Text>
            </View>
          </View>
          {card.cloudKey.quota > 0 ? (
            <View style={styles.usageMeterBlock}>
              <View style={styles.usageMeterHeader}>
                <Text style={styles.usageMeterLabel}>{text.quota}</Text>
                <Text style={styles.usageMeterValue}>
                  {formatUsd(card.cloudKey.quota_used)} / {formatUsd(card.cloudKey.quota)}
                </Text>
              </View>
              <View style={styles.usageMeterTrack}>
                <View
                  style={[
                    styles.usageMeterFillBase,
                    quotaRatio >= 0.8 && styles.usageMeterFillWarning,
                    quotaRatio >= 1 && styles.usageMeterFillDanger,
                    { width: `${Math.max(0, Math.min(quotaRatio, 1)) * 100}%` },
                  ]}
                />
              </View>
            </View>
          ) : null}
        </>
      ) : (
        <>
          <Text style={styles.routeSummaryProviderHint}>
            {text.routeUsageUnavailable(CLOUD_NAME)}
          </Text>
          <View style={styles.scopeActionsRow}>
            <Pressable
              onPress={onOpenRouting}
              style={({ pressed }) => [styles.primaryButton, pressed && styles.buttonPressed]}
              testID={`paseo-cloud-route-repair-routing-${actionId}`}
            >
              <Text style={styles.primaryButtonText}>{text.chooseMatchingGroup}</Text>
            </Pressable>
            <Pressable
              onPress={onOpenKeys}
              style={({ pressed }) => [styles.secondaryButton, pressed && styles.buttonPressed]}
              testID={`paseo-cloud-route-repair-keys-${actionId}`}
            >
              <Text style={styles.secondaryButtonText}>{text.createMatchingApiKey}</Text>
            </Pressable>
          </View>
        </>
      )}
    </View>
  );
}

function PaseoCloudOverviewSection(props: {
  isLoggedIn: boolean;
  canStartLogin: boolean;
  signedInAccountLabel: string;
  handleGitHubLogin: () => Promise<void>;
  handleLogout: () => Promise<void>;
  handleOpenPayModal: () => Promise<void>;
  meError: unknown;
  meIsLoading: boolean;
  meBalance: number | null | undefined;
  refetchMe: () => Promise<unknown>;
  usageTodayCost: number | null | undefined;
  usageTodayRequests: number | null | undefined;
  usageWeekCost: number | null | undefined;
  usageWeekRequests: number | null | undefined;
  usageMonthCost: number | null | undefined;
  usageMonthRequests: number | null | undefined;
  routeCards: RouteUsageCard[];
  onOpenKeys: () => void;
  onOpenRouting: () => void;
  onOpenCatalog: () => void;
  text: CloudPanelText;
}) {
  const { theme } = useUnistyles();
  const { text } = props;

  return (
    <>
      <SettingsSection title={text.accountTitle}>
        {!props.isLoggedIn ? (
          <View style={styles.dashedCard}>
            <View style={styles.emptyIconWrap}>
              <Cloud size={theme.iconSize.lg} color={theme.colors.foregroundMuted} />
            </View>
            <Text style={styles.emptyTitle}>{text.signIn}</Text>
            <Text style={styles.emptyBody}>{text.signInBody(CLOUD_NAME, APP_NAME)}</Text>
            <Pressable
              onPress={() => void props.handleGitHubLogin()}
              style={({ pressed }) => [
                styles.githubButton,
                pressed && styles.buttonPressed,
                !props.canStartLogin && styles.disabledButton,
              ]}
              disabled={!props.canStartLogin}
              testID="paseo-cloud-login-button"
            >
              <Text style={styles.githubButtonText}>{text.loginWithGitHub}</Text>
            </Pressable>
          </View>
        ) : (
          <View style={[settingsStyles.card, styles.cardBody]}>
            <Text style={styles.sectionHint}>{text.connectedHint(CLOUD_NAME)}</Text>
            <View style={styles.statusRow}>
              <View style={styles.statusBadge}>
                <View style={styles.statusDot} />
                <Text style={styles.statusText} numberOfLines={2}>
                  {text.signedInAs(props.signedInAccountLabel)}
                </Text>
              </View>
              <Pressable
                onPress={() => void props.handleLogout()}
                style={({ pressed }) => [styles.secondaryButton, pressed && styles.buttonPressed]}
              >
                <Text style={styles.secondaryButtonText}>{text.logout}</Text>
              </Pressable>
            </View>
          </View>
        )}
      </SettingsSection>

      {props.isLoggedIn ? (
        <SettingsSection title={text.cloudControlTitle}>
          <View style={[settingsStyles.card, styles.cardBody]}>
            <Text style={styles.sectionHint}>{text.cloudControlHint(CLOUD_NAME)}</Text>
            <View style={styles.scopeActionsRow}>
              <Pressable
                onPress={props.onOpenKeys}
                style={({ pressed }) => [styles.secondaryButton, pressed && styles.buttonPressed]}
                testID="paseo-cloud-overview-open-keys"
              >
                <Text style={styles.secondaryButtonText}>{text.createApiKey}</Text>
              </Pressable>
              <Pressable
                onPress={props.onOpenRouting}
                style={({ pressed }) => [styles.primaryButton, pressed && styles.buttonPressed]}
                testID="paseo-cloud-overview-open-routing"
              >
                <Text style={styles.primaryButtonText}>{text.switchGroup}</Text>
              </Pressable>
              <Pressable
                onPress={props.onOpenCatalog}
                style={({ pressed }) => [styles.secondaryButton, pressed && styles.buttonPressed]}
                testID="paseo-cloud-overview-open-catalog"
              >
                <Text style={styles.secondaryButtonText}>{text.viewModels}</Text>
              </Pressable>
            </View>
          </View>
        </SettingsSection>
      ) : null}

      {props.isLoggedIn ? (
        <SettingsSection title={text.balanceUsageTitle}>
          <View style={[settingsStyles.card, styles.cardBody]}>
            {props.meError ? (
              <View style={styles.errorBlock}>
                <Text style={styles.errorHint}>{getErrorMessage(props.meError)}</Text>
                <Pressable
                  onPress={() => void props.refetchMe()}
                  style={({ pressed }) => [styles.secondaryButton, pressed && styles.buttonPressed]}
                >
                  <Text style={styles.secondaryButtonText}>{text.retry}</Text>
                </Pressable>
              </View>
            ) : (
              <View style={styles.balanceHeader}>
                <View>
                  <Text style={styles.balanceLabel}>{text.balance}</Text>
                  <Text style={styles.balanceValue}>
                    {props.meIsLoading ? "…" : formatUsd(props.meBalance)}
                  </Text>
                </View>
                <Pressable
                  onPress={() => void props.handleOpenPayModal()}
                  style={({ pressed }) => [styles.primaryButton, pressed && styles.buttonPressed]}
                >
                  <Text style={styles.primaryButtonText}>{text.recharge}</Text>
                </Pressable>
              </View>
            )}
            <Text style={styles.usageHint}>
              {text.today}: {formatUsd(props.usageTodayCost)} ({props.usageTodayRequests ?? 0}{" "}
              {text.requests})
            </Text>
            <Text style={styles.usageHint}>
              {text.week}: {formatUsd(props.usageWeekCost)} ({props.usageWeekRequests ?? 0}{" "}
              {text.requests})
            </Text>
            <Text style={styles.usageHint}>
              {text.month}: {formatUsd(props.usageMonthCost)} ({props.usageMonthRequests ?? 0}{" "}
              {text.requests})
            </Text>
          </View>
        </SettingsSection>
      ) : null}

      {props.isLoggedIn ? (
        <SettingsSection title={text.currentRoutesTitle}>
          <View style={styles.routeSummaryGrid}>
            {props.routeCards.map((card) => (
              <RouteUsageCardBlock
                key={card.scopeLabel}
                card={card}
                text={text}
                onOpenKeys={props.onOpenKeys}
                onOpenRouting={props.onOpenRouting}
              />
            ))}
          </View>
        </SettingsSection>
      ) : null}
    </>
  );
}

export function PaseoCloudPanel({ initialSection }: { initialSection?: PaseoCloudSection } = {}) {
  const router = useRouter();
  const isCompact = useIsCompactFormFactor();
  const { settings } = useAppSettings();
  const { getAccessToken } = useSub2APIAuth();
  const { activeClaudeProvider, activeCodexProvider } = useDesktopProvidersStore();
  const [activeSection, setActiveSection] = useState<PaseoCloudSection>(
    initialSection ?? "overview",
  );
  const [isPayModalOpen, setIsPayModalOpen] = useState(false);
  const [payToken, setPayToken] = useState<string | null>(null);
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).cloudPanel, [locale]);
  const timezone = useMemo(() => getLocalTimeZone(), []);

  useEffect(() => {
    if (initialSection) {
      setActiveSection(initialSection);
    }
  }, [initialSection]);

  const meQuery = useSub2APIMe();
  const keysQuery = useSub2APIKeys(1, 200);
  const usageTodayQuery = useSub2APIUsageStats("today");
  const usageWeekQuery = useSub2APIUsageStats("week");
  const usageMonthQuery = useSub2APIUsageStats("month");
  const keys = useMemo(() => keysQuery.data?.items ?? [], [keysQuery.data?.items]);
  const activeClaudeCloudKey = useMemo(
    () => findCloudKeyByApiKey(keys, activeClaudeProvider?.apiKey),
    [activeClaudeProvider?.apiKey, keys],
  );
  const activeCodexCloudKey = useMemo(
    () => findCloudKeyByApiKey(keys, activeCodexProvider?.apiKey),
    [activeCodexProvider?.apiKey, keys],
  );
  const activeClaudeUsageTodayQuery = useSub2APIUsageStats(
    {
      period: "today",
      apiKeyId: activeClaudeCloudKey?.id ?? null,
      timezone,
    },
    { enabled: activeClaudeCloudKey !== null },
  );
  const activeCodexUsageTodayQuery = useSub2APIUsageStats(
    {
      period: "today",
      apiKeyId: activeCodexCloudKey?.id ?? null,
      timezone,
    },
    { enabled: activeCodexCloudKey !== null },
  );

  const {
    endpoint: serviceEndpoint,
    canStartLogin,
    isLoggedIn,
    auth,
    handleGitHubLogin,
    logout,
  } = useSub2APILoginFlow({
    defaultEndpoint: getManagedServiceUrlFromEnv(),
  });

  const signedInAccountLabel = useMemo(() => {
    const user = meQuery.data;
    if (user) {
      const name = user.username?.trim();
      if (name) {
        return name;
      }
      const email = user.email?.trim();
      if (email) {
        return email;
      }
    }
    if (meQuery.isPending || meQuery.isFetching) {
      return "…";
    }
    return text.accountFallback;
  }, [meQuery.data, meQuery.isFetching, meQuery.isPending, text.accountFallback]);

  const handleLogout = useCallback(async () => {
    await logout();
    if (settings.accessMode === "builtin") {
      router.replace("/login");
    }
  }, [logout, router, settings.accessMode]);

  const handleOpenPayModal = useCallback(async () => {
    try {
      const token = await getAccessToken();
      if (!token) {
        Alert.alert(text.sessionExpired, text.loginAgainBeforePayment);
        return;
      }
      setPayToken(token);
      setIsPayModalOpen(true);
    } catch (error) {
      Alert.alert(text.unableOpenPayment, getErrorMessage(error));
    }
  }, [getAccessToken, text]);

  const handlePayCompleted = useCallback(() => {
    void Promise.all([
      meQuery.refetch(),
      usageTodayQuery.refetch(),
      usageMonthQuery.refetch(),
      activeClaudeUsageTodayQuery.refetch(),
      activeCodexUsageTodayQuery.refetch(),
    ]);
  }, [
    activeClaudeUsageTodayQuery,
    activeCodexUsageTodayQuery,
    meQuery,
    usageMonthQuery,
    usageTodayQuery,
  ]);

  const routeCards = useMemo<RouteUsageCard[]>(
    () => [
      {
        scopeLabel: "Claude Code",
        provider: activeClaudeProvider ?? null,
        cloudKey: activeClaudeCloudKey,
        todayCost: activeClaudeUsageTodayQuery.data?.total_actual_cost,
        todayRequests: activeClaudeUsageTodayQuery.data?.total_requests,
      },
      {
        scopeLabel: "Codex",
        provider: activeCodexProvider ?? null,
        cloudKey: activeCodexCloudKey,
        todayCost: activeCodexUsageTodayQuery.data?.total_actual_cost,
        todayRequests: activeCodexUsageTodayQuery.data?.total_requests,
      },
    ],
    [
      activeClaudeCloudKey,
      activeClaudeProvider,
      activeClaudeUsageTodayQuery.data?.total_actual_cost,
      activeClaudeUsageTodayQuery.data?.total_requests,
      activeCodexCloudKey,
      activeCodexProvider,
      activeCodexUsageTodayQuery.data?.total_actual_cost,
      activeCodexUsageTodayQuery.data?.total_requests,
    ],
  );

  const activeSectionLabel = getSectionLabel(activeSection, text);

  const renderSection = () => {
    if (!isLoggedIn && activeSection !== "overview") {
      return (
        <SettingsSection title={activeSectionLabel}>
          <View style={styles.dashedCard}>
            <Text style={styles.emptyTitle}>{text.signInRequiredTitle}</Text>
            <Text style={styles.emptyBody}>
              {text.signInRequiredBody(getSectionLabel("overview", text))}
            </Text>
            <Pressable
              onPress={() => setActiveSection("overview")}
              style={({ pressed }) => [styles.primaryButton, pressed && styles.buttonPressed]}
            >
              <Text style={styles.primaryButtonText}>{text.goToOverview}</Text>
            </Pressable>
          </View>
        </SettingsSection>
      );
    }

    switch (activeSection) {
      case "keys":
        return (
          <PaseoCloudApiKeysSection
            authEndpoint={auth?.endpoint}
            serviceEndpoint={serviceEndpoint}
          />
        );
      case "routing":
        return (
          <PaseoCloudRoutingSection
            authEndpoint={auth?.endpoint}
            serviceEndpoint={serviceEndpoint}
          />
        );
      case "catalog":
        return <Sub2APIModelsSection />;
      case "usage":
        return <PaseoCloudUsageSection />;
      case "status":
        return <PaseoCloudModelStatusSection />;
      case "referral":
        return <PaseoCloudReferralSection />;
      case "overview":
      default:
        return (
          <PaseoCloudOverviewSection
            isLoggedIn={isLoggedIn}
            canStartLogin={canStartLogin}
            signedInAccountLabel={signedInAccountLabel}
            handleGitHubLogin={handleGitHubLogin}
            handleLogout={handleLogout}
            handleOpenPayModal={handleOpenPayModal}
            meError={meQuery.error}
            meIsLoading={meQuery.isLoading}
            meBalance={meQuery.data?.balance}
            refetchMe={meQuery.refetch}
            usageTodayCost={usageTodayQuery.data?.total_cost}
            usageTodayRequests={usageTodayQuery.data?.total_requests}
            usageWeekCost={usageWeekQuery.data?.total_cost}
            usageWeekRequests={usageWeekQuery.data?.total_requests}
            usageMonthCost={usageMonthQuery.data?.total_cost}
            usageMonthRequests={usageMonthQuery.data?.total_requests}
            routeCards={routeCards}
            onOpenKeys={() => setActiveSection("keys")}
            onOpenRouting={() => setActiveSection("routing")}
            onOpenCatalog={() => setActiveSection("catalog")}
            text={text}
          />
        );
    }
  };

  return (
    <>
      <View
        style={[styles.cloudShell, isCompact && styles.cloudShellCompact]}
        testID="paseo-cloud-panel"
      >
        <View
          style={[
            settingsStyles.card,
            styles.cloudMenuCard,
            isCompact && styles.cloudMenuCardCompact,
          ]}
        >
          <View style={styles.cloudMenuHeader}>
            <Text style={styles.formTitle}>{CLOUD_NAME}</Text>
            <Text style={styles.sectionHint}>{text.browseOneSection}</Text>
          </View>
          <View style={[styles.cloudMenuList, isCompact && styles.cloudMenuListCompact]}>
            {SECTION_OPTIONS.map((option) => {
              const selected = option.id === activeSection;
              return (
                <Pressable
                  key={option.id}
                  onPress={() => setActiveSection(option.id)}
                  style={({ pressed }) => [
                    styles.cloudMenuButton,
                    isCompact && styles.cloudMenuButtonCompact,
                    selected && styles.cloudMenuButtonActive,
                    pressed && !selected && styles.buttonPressed,
                  ]}
                  testID={option.testID}
                >
                  <Text
                    style={[
                      styles.cloudMenuButtonText,
                      selected && styles.cloudMenuButtonTextActive,
                    ]}
                  >
                    {getSectionLabel(option.id, text)}
                  </Text>
                </Pressable>
              );
            })}
          </View>
        </View>

        <View style={styles.cloudContent}>{renderSection()}</View>
      </View>

      {isLoggedIn ? (
        <Sub2APIPayModal
          visible={isPayModalOpen}
          endpoint={auth?.endpoint ?? serviceEndpoint}
          accessToken={payToken}
          onClose={() => setIsPayModalOpen(false)}
          onCompleted={handlePayCompleted}
        />
      ) : null}
    </>
  );
}
