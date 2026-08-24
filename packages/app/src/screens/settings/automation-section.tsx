import { useCallback, useMemo, useState } from "react";
import { Alert, Pressable, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { Plus, RotateCw } from "lucide-react-native";

import { SettingsSection } from "@/screens/settings/settings-section";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { StatusPanel } from "@/components/ui/status-panel";
import { StatusBadge } from "@/components/ui/status-badge";
import { Button } from "@/components/ui/button";
import { LoadingSpinner } from "@/components/ui/loading-spinner";
import { AdaptiveTextInput } from "@/components/adaptive-modal-sheet";
import { settingsStyles } from "@/styles/settings";
import { useLocalDaemonServerId } from "@/hooks/use-is-local-daemon";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { useAutomations, type AutomationSummary } from "@/hooks/use-automations";
import { useProvidersSnapshot } from "@/hooks/use-providers-snapshot";
import {
  defaultCadenceDraft,
  formatCadence,
  formatRunTimestamp,
  formatTimeOfDay,
  toCadence,
  type CadenceDraft,
  type CadenceKind,
} from "@/utils/schedule-format";

const CADENCE_KINDS: CadenceKind[] = ["interval", "daily", "weekdays", "weekly", "monthly"];

export function AutomationSection() {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.automation, [locale]);
  const serverId = useLocalDaemonServerId() ?? "";
  const {
    automations,
    isLoading,
    isFetching,
    error,
    isConnected,
    create,
    setPaused,
    remove,
    refetch,
  } = useAutomations(serverId);
  const { entries } = useProvidersSnapshot(serverId);

  const [isCreating, setIsCreating] = useState(false);
  const [name, setName] = useState("");
  const [prompt, setPrompt] = useState("");
  const [draft, setDraft] = useState<CadenceDraft>(() => defaultCadenceDraft("daily"));
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);

  const summaryText = useMemo(() => ({ ...text.summary, weekdayNames: text.weekdayNames }), [text]);

  const timezone = useMemo(() => {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone;
    } catch {
      return "UTC";
    }
  }, []);

  const readyProvider = useMemo(
    () => entries?.find((entry) => entry.status === "ready")?.provider ?? null,
    [entries],
  );

  const resetForm = useCallback(() => {
    setIsCreating(false);
    setName("");
    setPrompt("");
    setDraft(defaultCadenceDraft("daily"));
  }, []);

  const handleCreate = useCallback(() => {
    if (!prompt.trim()) {
      Alert.alert(text.promptRequired);
      return;
    }
    if (!readyProvider) {
      return;
    }
    create.mutate(
      {
        name: name.trim() || null,
        prompt: prompt.trim(),
        cadence: toCadence(draft),
        provider: readyProvider,
        // Runs in the daemon's own working directory; a per-automation directory
        // picker is a follow-up.
        cwd: ".",
      },
      {
        onSuccess: resetForm,
        onError: (mutationError) => {
          Alert.alert(
            text.loadFailed,
            mutationError instanceof Error ? mutationError.message : String(mutationError),
          );
        },
      },
    );
  }, [prompt, name, draft, readyProvider, create, resetForm, text]);

  const handleDelete = useCallback(
    (automation: AutomationSummary) => {
      // First press arms, second executes — no modal for a contextual destructive action.
      if (pendingDeleteId !== automation.id) {
        setPendingDeleteId(automation.id);
        return;
      }
      setPendingDeleteId(null);
      remove.mutate(automation.id);
    },
    [pendingDeleteId, remove],
  );

  const trailing = (
    <View style={styles.headerActions}>
      <Pressable
        onPress={() => void refetch()}
        disabled={isFetching}
        hitSlop={8}
        style={settingsStyles.sectionHeaderLink}
        accessibilityRole="button"
        accessibilityLabel={text.title}
      >
        {isFetching ? (
          <LoadingSpinner size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
        ) : (
          <RotateCw size={theme.iconSize.sm} color={theme.colors.foregroundMuted} />
        )}
      </Pressable>
      {!isCreating ? (
        <Button variant="outline" size="sm" leftIcon={Plus} onPress={() => setIsCreating(true)}>
          {text.create}
        </Button>
      ) : null}
    </View>
  );

  if (!isConnected) {
    return (
      <SettingsSection title={text.title}>
        <View style={settingsStyles.card}>
          <StatusPanel title={text.unsupported} />
        </View>
      </SettingsSection>
    );
  }

  return (
    <SettingsSection title={text.title} trailing={trailing}>
      <Text style={styles.subtitle}>{text.subtitle}</Text>

      {isCreating ? (
        <View style={settingsStyles.card}>
          <View style={styles.formSection}>
            <Text style={styles.fieldLabel}>{text.nameLabel}</Text>
            <AdaptiveTextInput
              value={name}
              onChangeText={setName}
              placeholder={text.namePlaceholder}
              placeholderTextColor={theme.colors.foregroundMuted}
              style={styles.input}
            />

            <Text style={styles.fieldLabel}>{text.promptLabel}</Text>
            <AdaptiveTextInput
              value={prompt}
              onChangeText={setPrompt}
              placeholder={text.promptPlaceholder}
              placeholderTextColor={theme.colors.foregroundMuted}
              multiline
              style={[styles.input, styles.textarea]}
            />

            <Text style={styles.fieldLabel}>{text.cadenceLabel}</Text>
            <SegmentedControl<CadenceKind>
              size="sm"
              value={draft.kind}
              onValueChange={(kind) => setDraft(defaultCadenceDraft(kind))}
              options={CADENCE_KINDS.map((kind) => ({ value: kind, label: text.kinds[kind] }))}
            />

            <CadenceEditor draft={draft} onChange={setDraft} text={text} />

            <Text style={styles.note}>{text.timezoneNote(timezone)}</Text>
            <Text style={styles.preview}>{formatCadence(toCadence(draft), summaryText)}</Text>

            <View style={styles.formActions}>
              <Button variant="ghost" size="sm" onPress={resetForm}>
                {text.cancel}
              </Button>
              <Button
                variant="default"
                size="sm"
                busy={create.isPending}
                disabled={!readyProvider}
                onPress={handleCreate}
              >
                {create.isPending ? text.creating : text.save}
              </Button>
            </View>
          </View>
        </View>
      ) : null}

      {error ? (
        <View style={settingsStyles.card}>
          <StatusPanel title={text.loadFailed} description={error} error />
        </View>
      ) : isLoading ? (
        <View style={settingsStyles.card}>
          <StatusPanel title={text.title} loading />
        </View>
      ) : automations.length === 0 ? (
        <View style={settingsStyles.card}>
          <StatusPanel title={text.empty} description={text.emptyHint} />
        </View>
      ) : (
        <View style={settingsStyles.card}>
          {automations.map((automation, index) => {
            const isPaused = automation.status === "paused";
            const nextRun = formatRunTimestamp(automation.nextRunAt, locale);
            const lastRun = formatRunTimestamp(automation.lastRunAt, locale);
            const isPendingDelete = pendingDeleteId === automation.id;

            return (
              <View
                key={automation.id}
                style={[settingsStyles.row, index > 0 && settingsStyles.rowBorder, styles.rowStack]}
              >
                <View style={styles.rowHeader}>
                  <Text style={settingsStyles.rowTitle} numberOfLines={1}>
                    {automation.name ?? automation.prompt}
                  </Text>
                  <StatusBadge
                    label={isPaused ? text.paused : text.active}
                    variant={isPaused ? "muted" : "success"}
                  />
                </View>
                <Text style={settingsStyles.rowHint} numberOfLines={1}>
                  {formatCadence(automation.cadence, summaryText)}
                  {nextRun && !isPaused ? ` · ${text.nextRun(nextRun)}` : ""}
                  {lastRun ? ` · ${text.lastRun(lastRun)}` : ""}
                </Text>
                <View style={styles.rowActions}>
                  <Button
                    variant="ghost"
                    size="sm"
                    busy={setPaused.isPending}
                    onPress={() => setPaused.mutate({ id: automation.id, paused: !isPaused })}
                  >
                    {isPaused ? text.resume : text.pause}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    busy={remove.isPending && isPendingDelete}
                    onPress={() => handleDelete(automation)}
                  >
                    {isPendingDelete ? text.confirmDelete : text.delete}
                  </Button>
                </View>
                {isPendingDelete ? (
                  <Text style={styles.deleteHint}>
                    {text.deleteHint(automation.name ?? automation.prompt)}
                  </Text>
                ) : null}
              </View>
            );
          })}
        </View>
      )}
    </SettingsSection>
  );
}

interface CadenceEditorProps {
  draft: CadenceDraft;
  onChange: (draft: CadenceDraft) => void;
  text: ReturnType<typeof getSub2APIMessages>["settings"]["automation"];
}

/** Each cadence kind gets the smallest editor it needs and nothing more. */
function CadenceEditor({ draft, onChange, text }: CadenceEditorProps) {
  const { theme } = useUnistyles();

  if (draft.kind === "interval") {
    return (
      <View style={styles.inlineRow}>
        <Text style={styles.inlineLabel}>{text.everyPrefix}</Text>
        <AdaptiveTextInput
          value={String(draft.intervalValue)}
          onChangeText={(value) =>
            onChange({ ...draft, intervalValue: Number.parseInt(value, 10) || 1 })
          }
          keyboardType="number-pad"
          placeholderTextColor={theme.colors.foregroundMuted}
          style={[styles.input, styles.numberInput]}
        />
        <SegmentedControl<"minutes" | "hours">
          size="sm"
          value={draft.intervalUnit}
          onValueChange={(intervalUnit) => onChange({ ...draft, intervalUnit })}
          options={[
            { value: "minutes", label: text.units.minutes },
            { value: "hours", label: text.units.hours },
          ]}
        />
      </View>
    );
  }

  return (
    <View style={styles.cadenceFields}>
      {draft.kind === "weekly" ? (
        <View style={styles.weekdayRow}>
          {text.weekdayNames.map((label, index) => {
            const isSelected = draft.weekday === index;
            return (
              <Pressable
                key={label}
                onPress={() => onChange({ ...draft, weekday: index })}
                style={[styles.weekdayChip, isSelected && styles.weekdayChipSelected]}
                accessibilityRole="button"
                accessibilityState={{ selected: isSelected }}
              >
                <Text style={[styles.weekdayText, isSelected && styles.weekdayTextSelected]}>
                  {label}
                </Text>
              </Pressable>
            );
          })}
        </View>
      ) : null}

      {draft.kind === "monthly" ? (
        <View style={styles.inlineRow}>
          <Text style={styles.inlineLabel}>{text.dayOfMonthLabel}</Text>
          <AdaptiveTextInput
            value={String(draft.dayOfMonth)}
            onChangeText={(value) =>
              onChange({ ...draft, dayOfMonth: Number.parseInt(value, 10) || 1 })
            }
            keyboardType="number-pad"
            placeholderTextColor={theme.colors.foregroundMuted}
            style={[styles.input, styles.numberInput]}
          />
          <Text style={styles.inlineLabel}>{text.dayOfMonthSuffix}</Text>
        </View>
      ) : null}

      <View style={styles.inlineRow}>
        <Text style={styles.inlineLabel}>{text.timeLabel}</Text>
        <AdaptiveTextInput
          value={String(draft.hour)}
          onChangeText={(value) =>
            onChange({ ...draft, hour: Math.min(23, Math.max(0, Number.parseInt(value, 10) || 0)) })
          }
          keyboardType="number-pad"
          placeholderTextColor={theme.colors.foregroundMuted}
          style={[styles.input, styles.numberInput]}
        />
        <Text style={styles.inlineLabel}>:</Text>
        <AdaptiveTextInput
          value={String(draft.minute).padStart(2, "0")}
          onChangeText={(value) =>
            onChange({
              ...draft,
              minute: Math.min(59, Math.max(0, Number.parseInt(value, 10) || 0)),
            })
          }
          keyboardType="number-pad"
          placeholderTextColor={theme.colors.foregroundMuted}
          style={[styles.input, styles.numberInput]}
        />
        <Text style={styles.inlineHint}>{formatTimeOfDay(draft.hour, draft.minute)}</Text>
      </View>
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  headerActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  subtitle: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
    marginBottom: theme.spacing[2],
  },
  formSection: {
    padding: theme.spacing[4],
    gap: theme.spacing[2],
  },
  fieldLabel: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
    marginTop: theme.spacing[1],
  },
  input: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface1,
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[2],
  },
  textarea: {
    minHeight: 88,
    textAlignVertical: "top",
  },
  numberInput: {
    width: 64,
    textAlign: "center",
  },
  cadenceFields: {
    gap: theme.spacing[2],
  },
  inlineRow: {
    flexDirection: "row",
    alignItems: "center",
    flexWrap: "wrap",
    gap: theme.spacing[2],
  },
  inlineLabel: {
    fontSize: theme.fontSize.sm,
    color: theme.colors.foreground,
  },
  inlineHint: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  weekdayRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[1],
  },
  weekdayChip: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.full,
    paddingHorizontal: theme.spacing[2],
    paddingVertical: 4,
  },
  weekdayChipSelected: {
    borderColor: theme.colors.accent,
    backgroundColor: theme.colors.surface3,
  },
  weekdayText: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  weekdayTextSelected: {
    color: theme.colors.foreground,
    fontWeight: theme.fontWeight.medium,
  },
  note: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  preview: {
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
    color: theme.colors.foreground,
  },
  formActions: {
    flexDirection: "row",
    justifyContent: "flex-end",
    gap: theme.spacing[2],
    marginTop: theme.spacing[2],
  },
  rowStack: {
    flexDirection: "column",
    alignItems: "stretch",
    gap: 4,
  },
  rowHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[2],
  },
  rowActions: {
    flexDirection: "row",
    gap: theme.spacing[1],
    marginTop: theme.spacing[1],
  },
  deleteHint: {
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
}));
