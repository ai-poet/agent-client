import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Dimensions, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { Button } from "@/components/ui/button";
import { useAppSettings } from "@/hooks/use-settings";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { useOnboardingGuideStore } from "@/stores/onboarding-guide-store";
import {
  type OnboardingGuideTargetId,
  type OnboardingGuideTargetRect,
  useOnboardingGuideTargetRegistry,
} from "@/components/onboarding-guide-target";

const SPOTLIGHT_PADDING = 8;
const PANEL_WIDTH = 360;
const PANEL_MARGIN = 16;
const FALLBACK_PANEL_TOP = 96;
const PANEL_ESTIMATED_HEIGHT = 330;
const MEASURE_DELAYS_MS = [0, 90, 240, 520] as const;

const TARGET_FALLBACKS: Partial<
  Record<OnboardingGuideTargetId, readonly OnboardingGuideTargetId[]>
> = {
  "sidebar.initializeGit": ["sidebar.newWorktree", "sidebar.projects"],
  "workspace.branchWorktree": ["workspace.branchSwitcher"],
  "changes.commit": ["agent.composer"],
};

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function isVisibleRect(
  rect: OnboardingGuideTargetRect | null,
  screen: { width: number; height: number },
): rect is OnboardingGuideTargetRect {
  if (!rect || rect.width <= 0 || rect.height <= 0) {
    return false;
  }
  return (
    rect.x < screen.width &&
    rect.y < screen.height &&
    rect.x + rect.width > 0 &&
    rect.y + rect.height > 0
  );
}

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
  const maxTop = Math.max(
    PANEL_MARGIN,
    input.screen.height - PANEL_ESTIMATED_HEIGHT - PANEL_MARGIN,
  );
  if (!input.rect) {
    return {
      left: Math.max(PANEL_MARGIN, (input.screen.width - width) / 2),
      top: clamp(FALLBACK_PANEL_TOP, PANEL_MARGIN, maxTop),
      width,
    };
  }

  const belowTop = input.rect.y + input.rect.height + PANEL_MARGIN;
  const aboveTop = input.rect.y - PANEL_ESTIMATED_HEIGHT - PANEL_MARGIN;
  const hasRoomBelow = belowTop + PANEL_ESTIMATED_HEIGHT < input.screen.height;
  const top = clamp(hasRoomBelow ? belowTop : aboveTop, PANEL_MARGIN, maxTop);
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
  fallbackDim: {
    backgroundColor: "rgba(0, 0, 0, 0.62)",
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
    flexWrap: "wrap",
    gap: theme.spacing[3],
  },
  footerGroup: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "flex-end",
    flexShrink: 1,
    flexWrap: "wrap",
    gap: theme.spacing[2],
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
    const targetIds = [step.targetId, ...(TARGET_FALLBACKS[step.targetId] ?? [])];
    const measure = () => {
      void Promise.all(targetIds.map((targetId) => registry.measure(targetId))).then((rects) => {
        if (!cancelled) {
          const rect = rects.find((candidate) => isVisibleRect(candidate, screen)) ?? null;
          setTargetRect(rect ? expandRect(rect, screen) : null);
        }
      });
    };

    setTargetRect(null);
    void Promise.all(targetIds.map((targetId) => registry.reveal(targetId))).catch(() => undefined);
    const timers = MEASURE_DELAYS_MS.map((delay) => setTimeout(measure, delay));
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
      {rect ? (
        <>
          <View style={[styles.dim, { top: 0, left: 0, right: 0, height: rect.y }]} />
          <View
            style={[
              styles.dim,
              {
                top: rect.y + rect.height,
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
                top: rect.y,
                left: 0,
                width: rect.x,
                height: rect.height,
              },
            ]}
          />
          <View
            style={[
              styles.dim,
              {
                top: rect.y,
                left: rect.x + rect.width,
                right: 0,
                height: rect.height,
              },
            ]}
          />
        </>
      ) : (
        <View style={[styles.dim, styles.fallbackDim, StyleSheet.absoluteFillObject]} />
      )}
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
