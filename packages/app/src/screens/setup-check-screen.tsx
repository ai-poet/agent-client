import { useCallback, useEffect, useMemo, useRef } from "react";
import { ActivityIndicator, Pressable, ScrollView, Text, View } from "react-native";
import { useRouter } from "expo-router";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { CheckCircle2, CircleAlert, MinusCircle } from "lucide-react-native";
import { PaseoLogo } from "@/components/icons/paseo-logo";
import { useAppSettings } from "@/hooks/use-settings";
import { useSetupChecks, type CheckItem } from "@/hooks/use-setup-checks";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";

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
    maxWidth: 560,
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
  checkList: {
    width: "100%",
    gap: theme.spacing[3],
  },
  checkCard: {
    flexDirection: "row",
    alignItems: "flex-start",
    padding: theme.spacing[4],
    borderRadius: theme.borderRadius.xl,
    backgroundColor: theme.colors.surface2,
    borderWidth: 1,
    borderColor: theme.colors.border,
    gap: theme.spacing[3],
  },
  checkIconBox: {
    width: 24,
    height: 24,
    alignItems: "center",
    justifyContent: "center",
    marginTop: 2,
  },
  checkBody: {
    flex: 1,
    gap: theme.spacing[1],
  },
  checkLabel: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.medium,
  },
  checkDesc: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  checkError: {
    color: theme.colors.destructive,
    fontSize: theme.fontSize.xs,
  },
  fixBtn: {
    alignSelf: "flex-start",
    marginTop: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[1],
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.accent,
  },
  fixBtnText: {
    color: theme.colors.accentForeground,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
  },
  actions: {
    width: "100%",
    alignItems: "center",
    gap: theme.spacing[3],
  },
  continueBtn: {
    width: "100%",
    paddingVertical: theme.spacing[3],
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.accent,
    alignItems: "center",
  },
  continueBtnDisabled: {
    opacity: 0.5,
  },
  continueBtnText: {
    color: theme.colors.accentForeground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.medium,
  },
  skipBtn: {
    paddingVertical: theme.spacing[2],
  },
  skipBtnText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
}));

function CheckIcon({ status }: { status: CheckItem["status"] }) {
  const { theme } = useUnistyles();
  switch (status) {
    case "pending":
      return <MinusCircle size={20} color={theme.colors.foregroundMuted} />;
    case "checking":
      return <ActivityIndicator size="small" color={theme.colors.accent} />;
    case "passed":
      return <CheckCircle2 size={20} color={theme.colors.success} />;
    case "failed":
      return <CircleAlert size={20} color={theme.colors.destructive} />;
    case "skipped":
      return <MinusCircle size={20} color={theme.colors.foregroundMuted} />;
  }
}

function CheckRow({ item, onFix }: { item: CheckItem; onFix: (id: CheckItem["id"]) => void }) {
  return (
    <View style={styles.checkCard}>
      <View style={styles.checkIconBox}>
        <CheckIcon status={item.status} />
      </View>
      <View style={styles.checkBody}>
        <Text style={styles.checkLabel}>{item.label}</Text>
        <Text style={styles.checkDesc}>{item.description}</Text>
        {item.status === "failed" && item.error && (
          <Text style={styles.checkError}>{item.error}</Text>
        )}
        {item.status === "failed" && item.fixLabel && (
          <Pressable style={styles.fixBtn} onPress={() => onFix(item.id)}>
            <Text style={styles.fixBtnText}>{item.fixLabel}</Text>
          </Pressable>
        )}
      </View>
    </View>
  );
}

export function SetupCheckScreen() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const { updateSettings } = useAppSettings();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).setupCheck, [locale]);
  const { checks, allPassed, isRunning, runAllChecks, fixCheck } = useSetupChecks();
  const autoTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Run checks on mount
  useEffect(() => {
    void runAllChecks();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Auto-continue when all passed
  useEffect(() => {
    if (autoTimerRef.current) {
      clearTimeout(autoTimerRef.current);
      autoTimerRef.current = null;
    }
    if (allPassed && !isRunning) {
      autoTimerRef.current = setTimeout(async () => {
        await updateSettings({ setupCheckCompleted: true });
        router.replace("/");
      }, 1500);
    }
    return () => {
      if (autoTimerRef.current) clearTimeout(autoTimerRef.current);
    };
  }, [allPassed, isRunning, router, updateSettings]);

  const handleContinue = useCallback(async () => {
    await updateSettings({ setupCheckCompleted: true });
    router.replace("/");
  }, [router, updateSettings]);

  const handleSkip = useCallback(async () => {
    await updateSettings({ setupCheckCompleted: true });
    router.replace("/");
  }, [router, updateSettings]);

  return (
    <View style={[styles.root, { paddingTop: insets.top, paddingBottom: insets.bottom }]}>
      <ScrollView style={styles.scrollView} contentContainerStyle={styles.container}>
        <View style={styles.content}>
          <PaseoLogo size={48} />
          <View style={styles.copyBlock}>
            <Text style={styles.title}>{text.title}</Text>
            <Text style={styles.subtitle}>{isRunning ? text.verifying : text.review}</Text>
          </View>

          <View style={styles.checkList}>
            {checks.map((item) => (
              <CheckRow key={item.id} item={item} onFix={fixCheck} />
            ))}
          </View>

          <View style={styles.actions}>
            <Pressable
              style={[styles.continueBtn, !allPassed && styles.continueBtnDisabled]}
              onPress={handleContinue}
              disabled={!allPassed}
            >
              <Text style={styles.continueBtnText}>{text.continue}</Text>
            </Pressable>
            <Pressable style={styles.skipBtn} onPress={handleSkip}>
              <Text style={styles.skipBtnText}>{text.skip}</Text>
            </Pressable>
          </View>
        </View>
      </ScrollView>
    </View>
  );
}
