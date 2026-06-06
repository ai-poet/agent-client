import { useEffect, useMemo } from "react";
import { View } from "react-native";
import { StyleSheet } from "react-native-unistyles";
import { WORKING_INDICATOR_CYCLE_MS, WORKING_INDICATOR_OFFSETS } from "@/utils/working-indicator";

const WEB_WORKING_DOTS_KEYFRAME_ID = "paseo-working-dots-keyframes";
const WEB_WORKING_DOTS_ANIMATION_NAME = "paseo-working-dot-bounce";
const WEB_WORKING_DOTS_KEYFRAME_CSS = `
  @keyframes ${WEB_WORKING_DOTS_ANIMATION_NAME} {
    0%, 100% {
      opacity: var(--paseo-working-dot-min-opacity, 0.3);
      transform: translateY(0);
    }
    50% {
      opacity: var(--paseo-working-dot-max-opacity, 1);
      transform: translateY(calc(var(--paseo-working-dot-lift, 6px) * -1));
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .paseo-working-dot {
      animation: none !important;
      opacity: var(--paseo-working-dot-min-opacity, 0.3) !important;
      transform: translateY(0) !important;
    }
  }
`;

let webWorkingDotsKeyframesRegistered = false;

function ensureWebWorkingDotsKeyframes() {
  if (typeof document === "undefined") {
    return;
  }
  const existing = document.getElementById(WEB_WORKING_DOTS_KEYFRAME_ID);
  if (existing) {
    if (existing.textContent !== WEB_WORKING_DOTS_KEYFRAME_CSS) {
      existing.textContent = WEB_WORKING_DOTS_KEYFRAME_CSS;
    }
    webWorkingDotsKeyframesRegistered = true;
    return;
  }
  if (webWorkingDotsKeyframesRegistered) {
    return;
  }
  const styleElement = document.createElement("style");
  styleElement.id = WEB_WORKING_DOTS_KEYFRAME_ID;
  styleElement.textContent = WEB_WORKING_DOTS_KEYFRAME_CSS;
  document.head.appendChild(styleElement);
  webWorkingDotsKeyframesRegistered = true;
}

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
  useEffect(() => {
    ensureWebWorkingDotsKeyframes();
  }, []);

  const dotStyle = useMemo(
    () =>
      ({
        width: dotSize,
        height: dotSize,
        borderRadius: dotSize / 2,
        backgroundColor: color,
        "--paseo-working-dot-lift": `${lift}px`,
        "--paseo-working-dot-min-opacity": minOpacity,
        "--paseo-working-dot-max-opacity": maxOpacity,
        animationName: WEB_WORKING_DOTS_ANIMATION_NAME,
        animationDuration: `${WORKING_INDICATOR_CYCLE_MS}ms`,
        animationTimingFunction: "ease-in-out",
        animationIterationCount: "infinite",
      }) as const,
    [color, dotSize, lift, maxOpacity, minOpacity],
  );

  return (
    <View style={styles.dotsRow} accessibilityRole="progressbar">
      {WORKING_INDICATOR_OFFSETS.map((offset, index) => (
        <View
          key={`working-dot-${index}`}
          className="paseo-working-dot"
          style={[
            styles.dot,
            dotStyle,
            {
              animationDelay: `${-offset * WORKING_INDICATOR_CYCLE_MS}ms`,
            } as any,
          ]}
        />
      ))}
    </View>
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
