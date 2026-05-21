import React, { useCallback, useMemo } from "react";
import { Pressable, Text, View } from "react-native";
import {
  BotMessageSquare,
  CheckCircle2,
  FolderOpen,
  GitBranchPlus,
  SlidersHorizontal,
} from "lucide-react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { AdaptiveModalSheet } from "@/components/adaptive-modal-sheet";
import { Button } from "@/components/ui/button";
import { useAppSettings } from "@/hooks/use-settings";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { useOnboardingGuideStore } from "@/stores/onboarding-guide-store";

const GUIDE_STEP_ICONS = [
  FolderOpen,
  BotMessageSquare,
  SlidersHorizontal,
  GitBranchPlus,
  CheckCircle2,
] as const;

const styles = StyleSheet.create((theme) => ({
  content: {
    gap: theme.spacing[6],
  },
  progressRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  progressDot: {
    height: 6,
    flex: 1,
    borderRadius: theme.borderRadius.full,
    backgroundColor: theme.colors.surface3,
  },
  progressDotActive: {
    backgroundColor: theme.colors.accent,
  },
  stepCard: {
    alignItems: "center",
    gap: theme.spacing[4],
    paddingVertical: theme.spacing[4],
  },
  iconBadge: {
    width: 56,
    height: 56,
    borderRadius: theme.borderRadius["2xl"],
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: theme.colors.surface2,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  stepCount: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
  },
  stepTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.lg,
    fontWeight: theme.fontWeight.medium,
    textAlign: "center",
  },
  stepBody: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    lineHeight: 20,
    textAlign: "center",
    maxWidth: 460,
  },
  footer: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[3],
  },
  footerGroup: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  skipButton: {
    alignSelf: "flex-start",
  },
  dotButton: {
    width: 24,
    height: 24,
    alignItems: "center",
    justifyContent: "center",
  },
  dotButtonInner: {
    width: 8,
    height: 8,
    borderRadius: 4,
    backgroundColor: theme.colors.surface3,
  },
  dotButtonInnerActive: {
    backgroundColor: theme.colors.accent,
  },
}));

export function OnboardingGuideDialog() {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).onboardingGuide, [locale]);
  const { updateSettings } = useAppSettings();
  const open = useOnboardingGuideStore((state) => state.open);
  const stepIndex = useOnboardingGuideStore((state) => state.stepIndex);
  const closeGuide = useOnboardingGuideStore((state) => state.closeGuide);
  const nextStep = useOnboardingGuideStore((state) => state.nextStep);
  const previousStep = useOnboardingGuideStore((state) => state.previousStep);
  const setStepIndex = useOnboardingGuideStore((state) => state.setStepIndex);
  const steps = text.steps;
  const safeStepIndex = Math.min(stepIndex, steps.length - 1);
  const step = steps[safeStepIndex] ?? steps[0];
  const Icon = GUIDE_STEP_ICONS[safeStepIndex] ?? GUIDE_STEP_ICONS[0];
  const isFirstStep = safeStepIndex === 0;
  const isLastStep = safeStepIndex === steps.length - 1;

  const completeGuide = useCallback(() => {
    void updateSettings({ onboardingGuideCompleted: true });
    closeGuide();
  }, [closeGuide, updateSettings]);

  const handleClose = useCallback(() => {
    closeGuide();
  }, [closeGuide]);

  const handlePrimary = useCallback(() => {
    if (isLastStep) {
      completeGuide();
      return;
    }
    nextStep(steps.length);
  }, [completeGuide, isLastStep, nextStep, steps.length]);

  if (!step) {
    return null;
  }

  return (
    <AdaptiveModalSheet
      title={text.title}
      visible={open}
      onClose={handleClose}
      testID="onboarding-guide-dialog"
      desktopMaxWidth={560}
      snapPoints={["70%", "92%"]}
    >
      <View style={styles.content} testID="onboarding-guide-content">
        <View style={styles.progressRow} accessibilityLabel={text.progressLabel}>
          {steps.map((item, index) => (
            <View
              key={item.title}
              style={[styles.progressDot, index <= safeStepIndex && styles.progressDotActive]}
            />
          ))}
        </View>

        <View style={styles.stepCard}>
          <View style={styles.iconBadge}>
            <Icon size={24} color={theme.colors.foreground} />
          </View>
          <Text style={styles.stepCount}>{text.stepCount(safeStepIndex + 1, steps.length)}</Text>
          <Text style={styles.stepTitle}>{step.title}</Text>
          <Text style={styles.stepBody}>{step.body}</Text>
        </View>

        <View style={styles.footer}>
          <Button
            variant="ghost"
            size="sm"
            onPress={completeGuide}
            style={styles.skipButton}
            testID="onboarding-guide-skip"
            accessibilityLabel={text.skipAccessibilityLabel}
          >
            {text.skip}
          </Button>
          <View style={styles.footerGroup}>
            <Button
              variant="secondary"
              size="sm"
              onPress={previousStep}
              disabled={isFirstStep}
              testID="onboarding-guide-previous"
            >
              {text.previous}
            </Button>
            {steps.map((item, index) => (
              <Pressable
                key={item.title}
                accessibilityRole="button"
                accessibilityLabel={text.goToStep(index + 1)}
                onPress={() => setStepIndex(index)}
                style={styles.dotButton}
                testID={`onboarding-guide-step-${index + 1}`}
              >
                <View
                  style={[
                    styles.dotButtonInner,
                    index === safeStepIndex && styles.dotButtonInnerActive,
                  ]}
                />
              </Pressable>
            ))}
            <Button
              variant="default"
              size="sm"
              onPress={handlePrimary}
              testID={isLastStep ? "onboarding-guide-finish" : "onboarding-guide-next"}
            >
              {isLastStep ? text.finish : text.next}
            </Button>
          </View>
        </View>
      </View>
    </AdaptiveModalSheet>
  );
}
