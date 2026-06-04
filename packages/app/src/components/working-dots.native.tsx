import { useEffect } from "react";
import { View } from "react-native";
import Animated, {
  cancelAnimation,
  Easing,
  type SharedValue,
  useAnimatedStyle,
  useSharedValue,
  withRepeat,
  withTiming,
} from "react-native-reanimated";
import { StyleSheet } from "react-native-unistyles";
import {
  getWorkingIndicatorDotStrength,
  WORKING_INDICATOR_CYCLE_MS,
  WORKING_INDICATOR_OFFSETS,
} from "@/utils/working-indicator";

type WorkingDotsProps = {
  color: string;
  dotSize?: number;
  lift?: number;
  minOpacity?: number;
  maxOpacity?: number;
};

export function WorkingDots({
  color,
  dotSize = 6,
  lift = 6,
  minOpacity = 0.3,
  maxOpacity = 1,
}: WorkingDotsProps) {
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
        <WorkingDot
          key={`working-dot-${index}`}
          color={color}
          dotSize={dotSize}
          lift={lift}
          maxOpacity={maxOpacity}
          minOpacity={minOpacity}
          offset={offset}
          progress={progress}
        />
      ))}
    </View>
  );
}

function WorkingDot({
  color,
  dotSize,
  lift,
  maxOpacity,
  minOpacity,
  offset,
  progress,
}: {
  color: string;
  dotSize: number;
  lift: number;
  maxOpacity: number;
  minOpacity: number;
  offset: number;
  progress: SharedValue<number>;
}) {
  const animatedStyle = useAnimatedStyle(() => {
    const strength = getWorkingIndicatorDotStrength(progress.value, offset);
    return {
      opacity: minOpacity + strength * (maxOpacity - minOpacity),
      transform: [{ translateY: strength * -lift }],
    };
  });

  return (
    <Animated.View
      style={[
        styles.dot,
        {
          width: dotSize,
          height: dotSize,
          borderRadius: dotSize / 2,
          backgroundColor: color,
        },
        animatedStyle,
      ]}
    />
  );
}

const styles = StyleSheet.create((theme) => ({
  dotsRow: {
    height: 16,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[1],
  },
  dot: {
    flexShrink: 0,
  },
}));
