import { useEffect, useState } from "react";
import { AccessibilityInfo } from "react-native";
import { getIsElectron } from "@/constants/platform";

type ReduceMotionSubscription = {
  remove?: () => void;
};

function getMotionInitialState(): boolean {
  return false;
}

export function useDesktopAgentMotionEnabled(): boolean {
  const [motionEnabled, setMotionEnabled] = useState(getMotionInitialState);

  useEffect(() => {
    if (!getIsElectron()) {
      setMotionEnabled(false);
      return;
    }

    let mounted = true;

    AccessibilityInfo.isReduceMotionEnabled()
      .then((reduceMotionEnabled) => {
        if (mounted) {
          setMotionEnabled(!reduceMotionEnabled);
        }
      })
      .catch(() => {
        if (mounted) {
          setMotionEnabled(true);
        }
      });

    const subscription = AccessibilityInfo.addEventListener?.(
      "reduceMotionChanged",
      (reduceMotionEnabled: boolean) => {
        setMotionEnabled(!reduceMotionEnabled);
      },
    ) as ReduceMotionSubscription | undefined;

    return () => {
      mounted = false;
      subscription?.remove?.();
    };
  }, []);

  return motionEnabled;
}
