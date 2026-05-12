import { useEffect, useMemo } from "react";
import { ActivityIndicator, Pressable, ScrollView, Text, View } from "react-native";
import { useRouter } from "expo-router";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { LogIn } from "lucide-react-native";
import { PaseoLogo } from "@/components/icons/paseo-logo";
import { useSub2APILoginFlow } from "@/hooks/use-sub2api-login-flow";
import { useAppSettings } from "@/hooks/use-settings";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import {
  getManagedServiceUrlFromEnv,
  hasExplicitManagedServiceUrlEnv,
  isManagedServiceUrlEnvValid,
} from "@/config/managed-service-env";
import { APP_NAME, CLOUD_NAME } from "@/config/branding";

const styles = StyleSheet.create((theme) => ({
  root: {
    flex: 1,
    backgroundColor: theme.colors.surface0,
  },
  scrollView: {
    flex: 1,
  },
  container: {
    flexGrow: 1,
    padding: theme.spacing[6],
    alignItems: "center",
    justifyContent: "center",
  },
  content: {
    width: "100%",
    maxWidth: 420,
    alignItems: "center",
    gap: theme.spacing[6],
  },
  copyBlock: {
    alignItems: "center",
    gap: theme.spacing[2],
  },
  title: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xl,
    fontWeight: theme.fontWeight.medium,
    textAlign: "center",
  },
  subtitle: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    textAlign: "center",
  },
  form: {
    width: "100%",
    gap: theme.spacing[3],
  },
  errorHint: {
    color: theme.colors.destructive,
    fontSize: theme.fontSize.xs,
  },
  primaryButton: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[2],
    paddingVertical: theme.spacing[4],
    borderRadius: theme.borderRadius.xl,
    backgroundColor: theme.colors.accent,
    borderWidth: 1,
    borderColor: theme.colors.accent,
  },
  primaryButtonDisabled: {
    opacity: 0.6,
  },
  primaryButtonPressed: {
    opacity: 0.85,
  },
  primaryButtonText: {
    color: theme.colors.accentForeground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.medium,
  },
  divider: {
    width: "100%",
    height: 1,
    backgroundColor: theme.colors.border,
  },
  byokRow: {
    alignItems: "center",
    gap: theme.spacing[1],
  },
  byokCaption: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    textAlign: "center",
  },
  byokLink: {
    paddingVertical: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
  },
  byokLinkText: {
    color: theme.colors.accent,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
  },
  waitingBlock: {
    width: "100%",
    alignItems: "center",
    gap: theme.spacing[3],
  },
  waitingRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[2],
  },
  waitingText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.medium,
  },
  retryAuthLink: {
    paddingVertical: theme.spacing[1],
    paddingHorizontal: theme.spacing[2],
  },
  retryAuthLinkText: {
    color: theme.colors.accent,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
    textDecorationLine: "underline",
  },
  retryAuthLinkDisabled: {
    opacity: 0.5,
  },
}));

export function LoginScreen() {
  const { theme } = useUnistyles();
  const insets = useSafeAreaInsets();
  const router = useRouter();
  const { updateSettings } = useAppSettings();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).loginScreen, [locale]);
  const injectedServiceUrl = getManagedServiceUrlFromEnv();
  const explicitServiceUrlEnv = hasExplicitManagedServiceUrlEnv();

  const { canStartLogin, isLoggedIn, handleGitHubLogin, isInFlight } = useSub2APILoginFlow({
    defaultEndpoint: injectedServiceUrl,
    onLoginSuccess: () => {
      router.replace("/");
    },
  });

  useEffect(() => {
    if (isLoggedIn) {
      router.replace("/");
    }
  }, [isLoggedIn, router]);

  const onSwitchToByok = async () => {
    await updateSettings({ accessMode: "byok", setupCheckCompleted: false });
    router.replace("/");
  };

  const envUrlInvalid = explicitServiceUrlEnv && !isManagedServiceUrlEnvValid();
  const loginDisabled = !canStartLogin || isInFlight || envUrlInvalid;

  return (
    <View style={styles.root}>
      <ScrollView
        style={styles.scrollView}
        contentContainerStyle={[
          styles.container,
          {
            paddingTop: theme.spacing[6] + insets.top,
            paddingBottom: theme.spacing[6] + insets.bottom,
          },
        ]}
        showsVerticalScrollIndicator={false}
        keyboardShouldPersistTaps="handled"
        testID="login-screen"
      >
        <View style={styles.content}>
          <PaseoLogo size={96} />
          <View style={styles.copyBlock}>
            <Text style={styles.title}>{text.title(APP_NAME)}</Text>
            <Text style={styles.subtitle}>{text.subtitle(CLOUD_NAME, APP_NAME)}</Text>
          </View>

          <View style={styles.form}>
            {envUrlInvalid ? <Text style={styles.errorHint}>{text.envUrlInvalid}</Text> : null}

            {isInFlight ? (
              <View style={styles.waitingBlock} testID="login-waiting-browser">
                <View style={styles.waitingRow}>
                  <ActivityIndicator size="small" color={theme.colors.accent} />
                  <Text style={styles.waitingText}>{text.waitingForBrowser}</Text>
                </View>
                <Pressable
                  onPress={() => void handleGitHubLogin()}
                  disabled={!canStartLogin || envUrlInvalid}
                  style={({ pressed }) => [
                    styles.retryAuthLink,
                    (!canStartLogin || envUrlInvalid) && styles.retryAuthLinkDisabled,
                    pressed && { opacity: 0.85 },
                  ]}
                  testID="login-retry-auth"
                >
                  <Text style={styles.retryAuthLinkText}>{text.requestAuthenticationAgain}</Text>
                </Pressable>
              </View>
            ) : (
              <Pressable
                onPress={() => void handleGitHubLogin()}
                disabled={loginDisabled}
                style={({ pressed }) => [
                  styles.primaryButton,
                  pressed && styles.primaryButtonPressed,
                  loginDisabled && styles.primaryButtonDisabled,
                ]}
                testID="login-managed-service-button"
              >
                <LogIn size={18} color={theme.colors.accentForeground} />
                <Text style={styles.primaryButtonText}>{text.primaryLogin}</Text>
              </Pressable>
            )}
          </View>

          <View style={styles.divider} />

          <View style={styles.byokRow}>
            <Text style={styles.byokCaption}>{text.byokCaption}</Text>
            <Pressable
              onPress={() => void onSwitchToByok()}
              style={styles.byokLink}
              testID="login-switch-byok"
            >
              <Text style={styles.byokLinkText}>{text.useByokInstead}</Text>
            </Pressable>
          </View>
        </View>
      </ScrollView>
    </View>
  );
}
