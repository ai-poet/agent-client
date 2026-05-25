import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Dimensions, Pressable, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { Button } from "@/components/ui/button";
import { useAppSettings } from "@/hooks/use-settings";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { useOnboardingGuideStore } from "@/stores/onboarding-guide-store";
import {
  type OnboardingGuideTargetRect,
  useOnboardingGuideTargetRegistry,
} from "@/components/onboarding-guide-target";

const SPOTLIGHT_PADDING = 8;
const PANEL_WIDTH = 360;
const PANEL_MARGIN = 16;
const FALLBACK_PANEL_TOP = 96;

function expandRect(
  rect: OnboardingGuideTargetRect,
  screen: { width: number; height: number },
): OnboardingGuideTargetRect {
  const x = Math.max(8, rect.x - SPOTLIGHT_PADDING);
  const y = Math.max(8, rect.y - SPOTLIGHT_PADDING);
  const right = Math.min(screen.width - 8, rect.x + rect.width + SPOTLIGHT_PADDING);
  const bottom = Math.min(screen.height - 8, rect.y + rect.height + SPOTLIGHT_PADDING);
  return {
    x,
    y,
    width: Math.max(0, right - x),
    height: Math.max(0, bottom - y),
  };
}

function computePanelPosition(input: {
  rect: OnboardingGuideTargetRect | null;
  screen: { width: number; height: number };
}): { left: number; top: number; width: number } {
  const width = Math.min(PANEL_WIDTH, input.screen.width - PANEL_MARGIN * 2);
  if (!input.rect) {
    return {
      left: Math.max(PANEL_MARGIN, (input.screen.width - width) / 2),
      top: Math.min(FALLBACK_PANEL_TOP, Math.max(PANEL_MARGIN, input.screen.height - 260)),
      width,
    };
  }

  const belowTop = input.rect.y + input.rect.height + PANEL_MARGIN;
  const aboveTop = input.rect.y - 220 - PANEL_MARGIN;
  const hasRoomBelow = belowTop + 220 < input.screen.height;
  const top = hasRoomBelow ? belowTop : Math.max(PANEL_MARGIN, aboveTop);
  const centerX = input.rect.x + input.rect.width / 2;
  const left = Math.max(
    PANEL_MARGIN,
    Math.min(input.screen.width - width - PANEL_MARGIN, centerX - width / 2),
  );
  return { left, top, width };
}

const styles = StyleSheet.create((theme) => ({
  overlay: {
    ...StyleSheet.absoluteFillObject,
    zIndex: 2000,
  },
  dim: {
    position: "absolute",
    backgroundColor: "rgba(0, 0, 0, 0.76)",
  },
  spotlightBorder: {
    position: "absolute",
    borderRadius: theme.borderRadius.lg,
    borderWidth: 2,
    borderColor: theme.colors.accent,
    backgroundColor: "rgba(255, 255, 255, 0.04)",
  },
  panel: {
    position: "absolute",
    gap: theme.spacing[4],
    padding: theme.spacing[4],
    borderRadius: theme.borderRadius.lg,
    borderWidth: theme.borderWidth[1],
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface0,
    ...theme.shadow.lg,
  },
  guideTitle: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.semibold,
    textTransform: "uppercase",
  },
  progressRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  progressDot: {
    height: 5,
    flex: 1,
    borderRadius: theme.borderRadius.full,
    backgroundColor: theme.colors.surface3,
  },
  progressDotActive: {
    backgroundColor: theme.colors.accent,
  },
  stepCount: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
  },
  title: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.lg,
    fontWeight: theme.fontWeight.semibold,
  },
  body: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    lineHeight: 20,
  },
  unavailable: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    lineHeight: 18,
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
  dotButton: {
    width: 18,
    height: 18,
    alignItems: "center",
    justifyContent: "center",
  },
  dotButtonInner: {
    width: 6,
    height: 6,
    borderRadius: 3,
    backgroundColor: theme.colors.surface3,
  },
  dotButtonInnerActive: {
    backgroundColor: theme.colors.accent,
  },
}));

export function OnboardingGuideDialog() {
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).onboardingGuide, [locale]);
  const { updateSettings } = useAppSettings();
  const registry = useOnboardingGuideTargetRegistry();
  const open = useOnboardingGuideStore((state) => state.open);
  const stepIndex = useOnboardingGuideStore((state) => state.stepIndex);
  const closeGuide = useOnboardingGuideStore((state) => state.closeGuide);
  const nextStep = useOnboardingGuideStore((state) => state.nextStep);
  const previousStep = useOnboardingGuideStore((state) => state.previousStep);
  const setStepIndex = useOnboardingGuideStore((state) => state.setStepIndex);
  const [targetRect, setTargetRect] = useState<OnboardingGuideTargetRect | null>(null);
  const [screen, setScreen] = useState(() => Dimensions.get("window"));

  const steps = text.steps;
  const safeStepIndex = Math.min(stepIndex, steps.length - 1);
  const step = steps[safeStepIndex] ?? steps[0];
  const isFirstStep = safeStepIndex === 0;
  const isLastStep = safeStepIndex === steps.length - 1;

  useEffect(() => {
    const subscription = Dimensions.addEventListener("change", ({ window }) => {
      setScreen(window);
    });
    return () => subscription.remove();
  }, []);

  useEffect(() => {
    if (!open || !step || !registry) {
      setTargetRect(null);
      return;
    }

    let cancelled = false;
    const measure = () => {
      void registry.measure(step.targetId).then((rect) => {
        if (!cancelled) {
          setTargetRect(rect ? expandRect(rect, screen) : null);
        }
      });
    };

    measure();
    const timers = [setTimeout(measure, 120), setTimeout(measure, 360)];
    return () => {
      cancelled = true;
      timers.forEach(clearTimeout);
    };
  }, [open, registry, screen, step]);

  const completeGuide = useCallback(() => {
    void updateSettings({ onboardingGuideCompleted: true });
    closeGuide();
  }, [closeGuide, updateSettings]);

  const handlePrimary = useCallback(() => {
    if (isLastStep) {
      completeGuide();
      return;
    }
    nextStep(steps.length);
  }, [completeGuide, isLastStep, nextStep, steps.length]);

  if (!open || !step) {
    return null;
  }

  const rect = targetRect;
  const panel = computePanelPosition({ rect, screen });

  return (
    <View style={styles.overlay} pointerEvents="auto" testID="onboarding-guide-dialog">
      <View style={[styles.dim, { top: 0, left: 0, right: 0, height: rect?.y ?? screen.height }]} />
      <View
        style={[
          styles.dim,
          {
            top: rect ? rect.y + rect.height : 0,
            left: 0,
            right: 0,
            bottom: 0,
          },
        ]}
      />
      <View
        style={[
          styles.dim,
          {
            top: rect?.y ?? 0,
            left: 0,
            width: rect?.x ?? screen.width,
            height: rect?.height ?? 0,
          },
        ]}
      />
      <View
        style={[
          styles.dim,
          {
            top: rect?.y ?? 0,
            left: rect ? rect.x + rect.width : 0,
            right: 0,
            height: rect?.height ?? 0,
          },
        ]}
      />
      {rect ? <View pointerEvents="none" style={[styles.spotlightBorder, rect] as any} /> : null}

      <View style={[styles.panel, panel] as any} testID="onboarding-guide-content">
        <Text style={styles.guideTitle}>{text.title}</Text>
        <View style={styles.progressRow} accessibilityLabel={text.progressLabel}>
          {steps.map((item, index) => (
            <View
              key={item.title}
              style={[styles.progressDot, index <= safeStepIndex && styles.progressDotActive]}
            />
          ))}
        </View>
        <Text style={styles.stepCount}>{text.stepCount(safeStepIndex + 1, steps.length)}</Text>
        <Text style={styles.title}>{step.title}</Text>
        <Text style={styles.body}>{step.body}</Text>
        {!rect ? <Text style={styles.unavailable}>{text.targetUnavailable}</Text> : null}

        <View style={styles.footer}>
          <Button
            variant="ghost"
            size="sm"
            onPress={completeGuide}
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
    </View>
  );
}
