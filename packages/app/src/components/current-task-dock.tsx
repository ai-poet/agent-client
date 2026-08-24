import { memo, useMemo, useState } from "react";
import { Pressable, ScrollView, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { ArrowRight, CheckCircle2, ChevronDown, Circle, ListChecks } from "lucide-react-native";

import { getAppMessages } from "@/i18n/sub2api";
import type { StreamItem } from "@/types/stream";
import { resolveActivePlan } from "@/utils/conversation-plan";

interface CurrentTaskDockProps {
  items: StreamItem[];
  locale?: string | null;
}

/**
 * Pinned progress for the plan the agent is currently working through. It disappears on
 * its own once every step is done, so it never lingers as stale context.
 */
export const CurrentTaskDock = memo(function CurrentTaskDock({
  items,
  locale,
}: CurrentTaskDockProps) {
  const { theme } = useUnistyles();
  const text = useMemo(() => getAppMessages(locale ?? "en").agentTools.dock, [locale]);
  const [isExpanded, setIsExpanded] = useState(false);

  const plan = useMemo(() => resolveActivePlan(items), [items]);
  if (!plan) {
    return null;
  }

  return (
    <View style={styles.container}>
      <Pressable
        onPress={() => setIsExpanded((previous) => !previous)}
        style={styles.header}
        accessibilityRole="button"
        accessibilityState={{ expanded: isExpanded }}
      >
        <ListChecks size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
        <Text style={styles.progress}>{text.progress(plan.completed, plan.total)}</Text>
        {plan.currentStep ? (
          <Text style={styles.currentStep} numberOfLines={1}>
            {plan.currentStep}
          </Text>
        ) : null}
        <ChevronDown
          size={theme.iconSize.sm}
          color={theme.colors.foregroundMuted}
          style={isExpanded ? styles.chevronExpanded : undefined}
        />
      </Pressable>

      {isExpanded ? (
        <ScrollView style={styles.list} showsVerticalScrollIndicator={false}>
          {plan.items.map((entry, index) => {
            const isCurrent = !entry.completed && entry.text === plan.currentStep;
            return (
              <View key={`${index}-${entry.text}`} style={styles.row}>
                {entry.completed ? (
                  <CheckCircle2 size={theme.iconSize.xs} color={theme.colors.statusSuccess} />
                ) : isCurrent ? (
                  <ArrowRight size={theme.iconSize.xs} color={theme.colors.accent} />
                ) : (
                  <Circle size={theme.iconSize.xs} color={theme.colors.foregroundMuted} />
                )}
                <Text
                  style={[styles.rowText, entry.completed && styles.rowTextDone]}
                  numberOfLines={2}
                >
                  {entry.text}
                </Text>
              </View>
            );
          })}
        </ScrollView>
      ) : null}
    </View>
  );
});

const styles = StyleSheet.create((theme) => ({
  container: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.surface2,
    marginBottom: theme.spacing[2],
    overflow: "hidden",
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[2],
  },
  progress: {
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
    color: theme.colors.foreground,
  },
  currentStep: {
    flex: 1,
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  chevronExpanded: {
    transform: [{ rotate: "180deg" }],
  },
  list: {
    // Capped so a long plan never crowds out the composer.
    maxHeight: 132,
    paddingHorizontal: theme.spacing[3],
    paddingBottom: theme.spacing[2],
  },
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: theme.spacing[2],
    paddingVertical: 3,
  },
  rowText: {
    flex: 1,
    fontSize: theme.fontSize.xs,
    color: theme.colors.foreground,
  },
  rowTextDone: {
    color: theme.colors.foregroundMuted,
    textDecorationLine: "line-through",
  },
}));
