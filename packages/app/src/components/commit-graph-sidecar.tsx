import { useEffect, useMemo, useRef } from "react";
import { StyleSheet as RNStyleSheet, useWindowDimensions, View } from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import Animated, { runOnJS, useAnimatedStyle, useSharedValue } from "react-native-reanimated";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { CommitGraphPane } from "@/components/commit-graph-pane";
import { isWeb } from "@/constants/platform";
import {
  DEFAULT_COMMIT_GRAPH_WIDTH,
  MAX_COMMIT_GRAPH_WIDTH,
  MIN_COMMIT_GRAPH_WIDTH,
  MIN_EXPLORER_SIDEBAR_WIDTH,
  usePanelStore,
} from "@/stores/panel-store";

const MIN_CENTER_WIDTH = 400;

interface CommitGraphSidecarProps {
  serverId: string;
  workspaceRoot: string;
  isOpen: boolean;
  explorerWidth: number;
  onClose: () => void;
}

export function CommitGraphSidecar({
  serverId,
  workspaceRoot,
  isOpen,
  explorerWidth,
  onClose,
}: CommitGraphSidecarProps) {
  const { theme } = useUnistyles();
  const insets = useSafeAreaInsets();
  const { width: viewportWidth } = useWindowDimensions();
  const graphWidth = usePanelStore((state) => state.commitGraphWidth);
  const setCommitGraphWidth = usePanelStore((state) => state.setCommitGraphWidth);

  const maxWidth = useMemo(() => {
    const available =
      viewportWidth - MIN_CENTER_WIDTH - Math.max(MIN_EXPLORER_SIDEBAR_WIDTH, explorerWidth);
    return Math.max(MIN_COMMIT_GRAPH_WIDTH, Math.min(MAX_COMMIT_GRAPH_WIDTH, available));
  }, [explorerWidth, viewportWidth]);

  useEffect(() => {
    if (!isOpen) {
      return;
    }
    if (graphWidth > maxWidth) {
      setCommitGraphWidth(maxWidth);
    } else if (graphWidth < MIN_COMMIT_GRAPH_WIDTH) {
      setCommitGraphWidth(DEFAULT_COMMIT_GRAPH_WIDTH);
    }
  }, [graphWidth, isOpen, maxWidth, setCommitGraphWidth]);

  const startWidthRef = useRef(graphWidth);
  const resizeWidth = useSharedValue(
    Math.min(Math.max(graphWidth, MIN_COMMIT_GRAPH_WIDTH), maxWidth),
  );

  useEffect(() => {
    resizeWidth.value = Math.min(Math.max(graphWidth, MIN_COMMIT_GRAPH_WIDTH), maxWidth);
  }, [graphWidth, maxWidth, resizeWidth]);

  const resizeGesture = useMemo(
    () =>
      Gesture.Pan()
        .enabled(isOpen)
        .hitSlop({ left: 8, right: 8, top: 0, bottom: 0 })
        .onStart(() => {
          startWidthRef.current = graphWidth;
          resizeWidth.value = graphWidth;
        })
        .onUpdate((event) => {
          const newWidth = startWidthRef.current - event.translationX;
          const clampedWidth = Math.max(MIN_COMMIT_GRAPH_WIDTH, Math.min(maxWidth, newWidth));
          resizeWidth.value = clampedWidth;
        })
        .onEnd(() => {
          runOnJS(setCommitGraphWidth)(resizeWidth.value);
        }),
    [graphWidth, isOpen, maxWidth, resizeWidth, setCommitGraphWidth],
  );

  const animatedStyle = useAnimatedStyle(() => ({
    width: resizeWidth.value,
  }));

  if (!isOpen) {
    return null;
  }

  return (
    <Animated.View
      testID="workspace-commit-graph-sidecar"
      style={[staticStyles.sidecar, animatedStyle, { paddingTop: insets.top }]}
    >
      <View style={[styles.border, { backgroundColor: theme.colors.background }]}>
        <GestureDetector gesture={resizeGesture}>
          <View style={[styles.resizeHandle, isWeb && ({ cursor: "col-resize" } as any)]} />
        </GestureDetector>
        <CommitGraphPane serverId={serverId} cwd={workspaceRoot} onClose={onClose} />
      </View>
    </Animated.View>
  );
}

const staticStyles = RNStyleSheet.create({
  sidecar: {
    position: "relative",
    flexShrink: 0,
  },
});

const styles = StyleSheet.create((theme) => ({
  border: {
    flex: 1,
    borderLeftWidth: 1,
    borderLeftColor: theme.colors.border,
  },
  resizeHandle: {
    position: "absolute",
    left: -5,
    top: 0,
    bottom: 0,
    width: 10,
    zIndex: 10,
  },
}));
