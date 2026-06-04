import { useEffect } from "react";
import { ActivityIndicator, Text, View } from "react-native";
import Animated, {
  cancelAnimation,
  Easing,
  type SharedValue,
  useAnimatedStyle,
  useSharedValue,
  withRepeat,
  withTiming,
} from "react-native-reanimated";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { useDesktopAgentMotionEnabled } from "@/hooks/use-desktop-agent-motion-enabled";
import {
  getWorkingIndicatorDotStrength,
  WORKING_INDICATOR_CYCLE_MS,
  WORKING_INDICATOR_OFFSETS,
} from "@/utils/working-indicator";

type DesktopAgentLoadingStateProps = {
  title?: string;
  subtitle?: string;
  tone?: "neutral" | "blocking";
};

export function DesktopAgentLoadingState({
  title,
  subtitle,
  tone = "neutral",
}: DesktopAgentLoadingStateProps) {
  const { theme } = useUnistyles();
  const motionEnabled = useDesktopAgentMotionEnabled();
  const dotColor = tone === "blocking" ? theme.colors.foreground : theme.colors.foregroundMuted;

  return (
    <View style={styles.container} testID="desktop-agent-loading-state">
      {motionEnabled ? (
        <DesktopWorkingDots color={dotColor} />
      ) : (
        <ActivityIndicator size="small" color={dotColor} />
      )}
      {title ? <Text style={styles.title}>{title}</Text> : null}
      {subtitle ? <Text style={styles.subtitle}>{subtitle}</Text> : null}
    </View>
  );
}

function DesktopWorkingDots({ color }: { color: string }) {
  const progress = useSharedValue(0);

  useEffect(() => {
    progress.value = 0;
    progress.value = withRepeat(
      withTiming(1, {
        duration: WORKING_INDICATOR_CYCLE_MS,
        easing: Easing.linear,
      }),
      -1,
      false,
    );

    return () => {
      cancelAnimation(progress);
      progress.value = 0;
    };
  }, [progress]);

  return (
    <View style={styles.dotsRow} accessibilityRole="progressbar">
      {WORKING_INDICATOR_OFFSETS.map((offset, index) => (
        <DesktopWorkingDot
          key={`desktop-working-dot-${index}`}
          color={color}
          offset={offset}
          progress={progress}
        />
      ))}
    </View>
  );
}

function DesktopWorkingDot({
  color,
  offset,
  progress,
}: {
  color: string;
  offset: number;
  progress: SharedValue<number>;
}) {
  const animatedStyle = useAnimatedStyle(() => {
    const strength = getWorkingIndicatorDotStrength(progress.value, offset);
    return {
      opacity: 0.35 + strength * 0.55,
      transform: [{ translateY: strength * -5 }],
    };
  });

  return <Animated.View style={[styles.dot, { backgroundColor: color }, animatedStyle]} />;
}

const styles = StyleSheet.create((theme) => ({
  container: {
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[6],
  },
  dotsRow: {
    height: 16,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[1],
  },
  dot: {
    width: 5,
    height: 5,
    borderRadius: theme.borderRadius.full,
  },
  title: {
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.semibold,
    color: theme.colors.foreground,
    textAlign: "center",
  },
  subtitle: {
    maxWidth: 360,
    fontSize: theme.fontSize.sm,
    color: theme.colors.foregroundMuted,
    textAlign: "center",
  },
}));
