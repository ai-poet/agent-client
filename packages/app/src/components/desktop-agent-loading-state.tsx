import { ActivityIndicator, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { WorkingDots } from "@/components/working-dots";
import { useDesktopAgentMotionEnabled } from "@/hooks/use-desktop-agent-motion-enabled";

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
        <WorkingDots color={dotColor} dotSize={5} lift={5} minOpacity={0.35} maxOpacity={0.9} />
      ) : (
        <ActivityIndicator size="small" color={dotColor} />
      )}
      {title ? <Text style={styles.title}>{title}</Text> : null}
      {subtitle ? <Text style={styles.subtitle}>{subtitle}</Text> : null}
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  container: {
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[6],
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
