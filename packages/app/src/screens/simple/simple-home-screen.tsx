import { useCallback, useMemo, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  Text,
  TextInput,
  View,
} from "react-native";
import { useRouter } from "expo-router";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { FolderOpen, RefreshCw, Send, Settings, SlidersHorizontal } from "lucide-react-native";
import { DraftAgentStatusBar } from "@/components/agent-status-bar";
import { Button } from "@/components/ui/button";
import { getProviderIcon } from "@/components/provider-icons";
import { useAllAgentsList } from "@/hooks/use-all-agents-list";
import { useAgentInputDraft } from "@/hooks/use-agent-input-draft";
import { useAppSettings } from "@/hooks/use-settings";
import { useIsLocalDaemon } from "@/hooks/use-is-local-daemon";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import {
  useHostRuntimeClient,
  useHostRuntimeIsConnected,
} from "@/runtime/host-runtime";
import { useRecommendedProjectPaths } from "@/stores/session-store-hooks";
import { useSessionStore } from "@/stores/session-store";
import { pickDirectory } from "@/desktop/pick-directory";
import { splitComposerAttachmentsForSubmit } from "@/components/composer-attachments";
import { generateMessageId } from "@/types/stream";
import { normalizeAgentSnapshot } from "@/utils/agent-snapshots";
import { encodeImages } from "@/utils/encode-images";
import { formatTimeAgo } from "@/utils/time";
import { shortenPath } from "@/utils/shorten-path";
import { buildHostSimpleAgentRoute, buildSettingsRoute } from "@/utils/host-routes";
import { useToast } from "@/contexts/toast-context";
import type { AggregatedAgent } from "@/hooks/use-aggregated-agents";
import type { AgentSessionConfig } from "@server/server/agent/agent-sdk-types";
import { isSimpleModeAgent, SIMPLE_EXPERIENCE_LABEL } from "./simple-mode";

function toErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function SimpleHomeScreen({ serverId }: { serverId: string }) {
  const { theme } = useUnistyles();
  const insets = useSafeAreaInsets();
  const router = useRouter();
  const toast = useToast();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).simpleMode, [locale]);
  const { settings, updateSettings } = useAppSettings();
  const client = useHostRuntimeClient(serverId);
  const isConnected = useHostRuntimeIsConnected(serverId);
  const isLocalDaemon = useIsLocalDaemon(serverId);
  const recommendedFolders = useRecommendedProjectPaths(serverId);
  const { agents, isRevalidating, refreshAll } = useAllAgentsList({
    serverId,
    includeArchived: false,
  });
  const setAgents = useSessionStore((state) => state.setAgents);
  const [manualFolder, setManualFolder] = useState("");
  const [isManualFolderOpen, setIsManualFolderOpen] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [isCreating, setIsCreating] = useState(false);

  const simpleAgents = useMemo(() => agents.filter(isSimpleModeAgent), [agents]);
  const recentTaskFolder = simpleAgents.find((agent) => agent.cwd.trim().length > 0)?.cwd ?? "";
  const savedTaskFolder = settings.simpleTaskFolder.trim();
  const taskFolder =
    savedTaskFolder ||
    recommendedFolders.find((folder) => folder.trim().length > 0) ||
    recentTaskFolder;

  const draftInput = useAgentInputDraft({
    draftKey: `simple:new:${serverId}`,
    initialCwd: taskFolder,
    composer: {
      initialServerId: serverId,
      initialValues: taskFolder ? { workingDir: taskFolder } : undefined,
      isVisible: true,
      onlineServerIds: isConnected ? [serverId] : [],
      lockedWorkingDir: taskFolder || undefined,
    },
  });
  const composerState = draftInput.composerState;
  const selectedProvider = composerState?.selectedProvider ?? null;
  const providerDefinition = composerState?.providerDefinitions.find(
    (definition) => definition.id === selectedProvider,
  );
  const modelSummary = selectedProvider
    ? text.modelSummary(
        providerDefinition?.label ?? selectedProvider,
        composerState?.effectiveModelId || text.defaultModel,
      )
    : text.modelNotReady;
  const canSubmit =
    draftInput.text.trim().length > 0 &&
    taskFolder.trim().length > 0 &&
    Boolean(client) &&
    isConnected &&
    Boolean(selectedProvider) &&
    !isCreating;

  const chooseFolder = useCallback(async () => {
    try {
      if (!isLocalDaemon) {
        setManualFolder(taskFolder);
        setIsManualFolderOpen(true);
        return;
      }
      const nextFolder = await pickDirectory();
      if (!nextFolder) {
        return;
      }
      await updateSettings({ simpleTaskFolder: nextFolder });
    } catch (error) {
      toast.error(toErrorMessage(error));
    }
  }, [isLocalDaemon, taskFolder, toast, updateSettings]);

  const saveManualFolder = useCallback(async () => {
    const nextFolder = manualFolder.trim();
    if (!nextFolder) {
      toast.error(text.errors.chooseFolder);
      return;
    }
    await updateSettings({ simpleTaskFolder: nextFolder });
    setIsManualFolderOpen(false);
  }, [manualFolder, text.errors.chooseFolder, toast, updateSettings]);

  const createTask = useCallback(async () => {
    const prompt = draftInput.text.trim();
    const cwd = taskFolder.trim();
    if (!cwd) {
      toast.error(text.errors.chooseFolder);
      return;
    }
    if (!client || !isConnected) {
      toast.error(text.errors.hostNotConnected);
      return;
    }
    if (!composerState || !composerState.selectedProvider) {
      toast.error(text.errors.selectModel);
      return;
    }

    try {
      setIsCreating(true);
      const modeId =
        composerState.modeOptions.length > 0 && composerState.selectedMode !== ""
          ? composerState.selectedMode
          : undefined;
      const config: AgentSessionConfig = {
        provider: composerState.selectedProvider,
        cwd,
        ...(modeId ? { modeId } : {}),
        ...(composerState.effectiveModelId ? { model: composerState.effectiveModelId } : {}),
        ...(composerState.effectiveThinkingOptionId
          ? { thinkingOptionId: composerState.effectiveThinkingOptionId }
          : {}),
      };
      const { images, attachments } = splitComposerAttachmentsForSubmit(draftInput.attachments);
      const encodedImages = await encodeImages(images);
      const result = await client.createAgent({
        config,
        initialPrompt: prompt,
        clientMessageId: generateMessageId(),
        labels: { experienceMode: SIMPLE_EXPERIENCE_LABEL },
        ...(encodedImages && encodedImages.length > 0 ? { images: encodedImages } : {}),
        ...(attachments.length > 0 ? { attachments } : {}),
      });

      setAgents(serverId, (previous) => {
        const next = new Map(previous);
        next.set(result.id, normalizeAgentSnapshot(result, serverId));
        return next;
      });
      await composerState.persistFormPreferences();
      draftInput.clear("sent");
      router.push(buildHostSimpleAgentRoute(serverId, result.id));
    } catch (error) {
      toast.error(toErrorMessage(error) || text.errors.failedCreate);
    } finally {
      setIsCreating(false);
    }
  }, [
    client,
    composerState,
    draftInput,
    isConnected,
    router,
    serverId,
    setAgents,
    taskFolder,
    text.errors.chooseFolder,
    text.errors.failedCreate,
    text.errors.hostNotConnected,
    text.errors.selectModel,
    toast,
  ]);

  return (
    <View style={styles.root}>
      <View
        style={[
          styles.header,
          {
            paddingTop: theme.spacing[3] + insets.top,
          },
        ]}
      >
        <View>
          <Text style={styles.headerTitle}>{text.tasks}</Text>
          <Text style={styles.headerSubtitle}>{modelSummary}</Text>
        </View>
        <View style={styles.headerActions}>
          <Button
            size="sm"
            variant="ghost"
            leftIcon={RefreshCw}
            onPress={refreshAll}
            disabled={isRevalidating}
          >
            {text.refresh}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            leftIcon={Settings}
            onPress={() => router.push(buildSettingsRoute())}
          >
            {text.settings}
          </Button>
        </View>
      </View>

      <ScrollView
        style={styles.scrollView}
        contentContainerStyle={[
          styles.content,
          {
            paddingBottom: theme.spacing[6] + insets.bottom,
          },
        ]}
      >
        <View style={styles.promptPanel}>
          <View style={styles.promptHeader}>
            <Text style={styles.sectionTitle}>{text.newTask}</Text>
            <Button
              size="sm"
              variant="ghost"
              leftIcon={SlidersHorizontal}
              onPress={() => setShowAdvanced((value) => !value)}
            >
              {showAdvanced ? text.hideAdvanced : text.advanced}
            </Button>
          </View>
          <TextInput
            multiline
            value={draftInput.text}
            onChangeText={draftInput.setText}
            placeholder={text.promptPlaceholder}
            placeholderTextColor={theme.colors.foregroundMuted}
            style={styles.promptInput}
            testID="simple-new-task-input"
          />
          {showAdvanced && composerState ? (
            <View style={styles.advancedControls}>
              <DraftAgentStatusBar
                {...composerState.statusControls}
                serverId={serverId}
                cwd={taskFolder}
              />
            </View>
          ) : null}
          <View style={styles.promptFooter}>
            <Pressable style={styles.folderButton} onPress={chooseFolder}>
              <FolderOpen size={16} color={theme.colors.foregroundMuted} />
              <Text style={styles.folderButtonText} numberOfLines={1}>
                {taskFolder ? shortenPath(taskFolder) : text.chooseFolderTitle}
              </Text>
            </Pressable>
            <Button
              size="md"
              variant="default"
              leftIcon={isCreating ? null : Send}
              onPress={() => void createTask()}
              disabled={!canSubmit}
              testID="simple-create-task"
            >
              {isCreating ? text.creating : text.startTask}
            </Button>
          </View>
          {!taskFolder || isManualFolderOpen ? (
            <View style={styles.folderCallout} testID="simple-folder-cta">
              <Text style={styles.folderCalloutTitle}>{text.chooseFolderTitle}</Text>
              <Text style={styles.folderCalloutHint}>{text.chooseFolderHint}</Text>
              <View style={styles.manualFolderRow}>
                <TextInput
                  value={manualFolder}
                  onChangeText={setManualFolder}
                  placeholder={text.folderPathPlaceholder}
                  placeholderTextColor={theme.colors.foregroundMuted}
                  style={styles.folderInput}
                />
                <Button size="sm" variant="secondary" onPress={() => void saveManualFolder()}>
                  {text.useThisFolder}
                </Button>
              </View>
            </View>
          ) : null}
        </View>

        <View style={styles.tasksSection}>
          <Text style={styles.sectionTitle}>{text.tasks}</Text>
          {simpleAgents.length === 0 ? (
            <View style={styles.emptyState}>
              <Text style={styles.emptyTitle}>{text.noTasksTitle}</Text>
              <Text style={styles.emptyHint}>{text.noTasksHint}</Text>
            </View>
          ) : (
            <View style={styles.taskList} testID="simple-task-list">
              {simpleAgents.map((agent) => (
                <SimpleTaskRow key={`${agent.serverId}:${agent.id}`} agent={agent} />
              ))}
            </View>
          )}
        </View>
      </ScrollView>
    </View>
  );
}

function SimpleTaskRow({ agent }: { agent: AggregatedAgent }) {
  const { theme } = useUnistyles();
  const router = useRouter();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).simpleMode, [locale]);
  const ProviderIcon = getProviderIcon(agent.provider);

  return (
    <Pressable
      style={({ pressed }) => [styles.taskRow, pressed ? styles.pressed : null]}
      onPress={() => router.push(buildHostSimpleAgentRoute(agent.serverId, agent.id))}
      testID={`simple-task-row-${agent.id}`}
    >
      <View style={styles.taskIcon}>
        <ProviderIcon size={18} color={theme.colors.foreground} />
      </View>
      <View style={styles.taskText}>
        <Text style={styles.taskTitle} numberOfLines={1}>
          {agent.title || text.untitledTask}
        </Text>
        <Text style={styles.taskMeta} numberOfLines={1}>
          {text.status[agent.status]} · {shortenPath(agent.cwd)} ·{" "}
          {formatTimeAgo(agent.lastActivityAt)}
        </Text>
      </View>
      {(agent.pendingPermissionCount ?? 0) > 0 ? (
        <View style={styles.permissionBadge}>
          <Text style={styles.permissionBadgeText}>{text.needsApproval}</Text>
        </View>
      ) : null}
      {agent.status === "running" ? (
        <ActivityIndicator size="small" color={theme.colors.accent} />
      ) : null}
    </Pressable>
  );
}

const styles = StyleSheet.create((theme) => ({
  root: {
    flex: 1,
    backgroundColor: theme.colors.surface0,
  },
  header: {
    minHeight: 72,
    paddingHorizontal: theme.spacing[6],
    paddingBottom: theme.spacing[3],
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
    backgroundColor: theme.colors.surface0,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[4],
  },
  headerTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xl,
    fontWeight: theme.fontWeight.medium,
  },
  headerSubtitle: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    marginTop: theme.spacing[1],
  },
  headerActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  scrollView: {
    flex: 1,
  },
  content: {
    width: "100%",
    maxWidth: 920,
    alignSelf: "center",
    padding: theme.spacing[6],
    gap: theme.spacing[6],
  },
  promptPanel: {
    borderRadius: theme.borderRadius.lg,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
    padding: theme.spacing[4],
    gap: theme.spacing[3],
  },
  promptHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[3],
  },
  sectionTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.medium,
  },
  promptInput: {
    minHeight: 148,
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    lineHeight: theme.fontSize.base * 1.45,
    textAlignVertical: "top",
    borderRadius: theme.borderRadius.md,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface0,
    padding: theme.spacing[4],
  },
  advancedControls: {
    borderTopWidth: 1,
    borderTopColor: theme.colors.border,
    paddingTop: theme.spacing[3],
  },
  promptFooter: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[3],
  },
  folderButton: {
    flex: 1,
    minHeight: 42,
    borderRadius: theme.borderRadius.md,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface0,
    paddingHorizontal: theme.spacing[3],
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  folderButtonText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    flex: 1,
  },
  folderCallout: {
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface2,
    padding: theme.spacing[3],
    gap: theme.spacing[2],
  },
  folderCalloutTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
  },
  folderCalloutHint: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    lineHeight: theme.fontSize.xs * 1.5,
  },
  manualFolderRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  folderInput: {
    flex: 1,
    minHeight: 38,
    borderRadius: theme.borderRadius.md,
    borderWidth: 1,
    borderColor: theme.colors.border,
    color: theme.colors.foreground,
    backgroundColor: theme.colors.surface0,
    paddingHorizontal: theme.spacing[3],
    fontSize: theme.fontSize.sm,
  },
  tasksSection: {
    gap: theme.spacing[3],
  },
  emptyState: {
    borderRadius: theme.borderRadius.lg,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
    padding: theme.spacing[6],
    alignItems: "center",
    gap: theme.spacing[2],
  },
  emptyTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.medium,
  },
  emptyHint: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    textAlign: "center",
  },
  taskList: {
    borderRadius: theme.borderRadius.lg,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
    overflow: "hidden",
  },
  taskRow: {
    minHeight: 64,
    paddingHorizontal: theme.spacing[4],
    paddingVertical: theme.spacing[3],
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[3],
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  pressed: {
    opacity: 0.82,
  },
  taskIcon: {
    width: 36,
    height: 36,
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface2,
    alignItems: "center",
    justifyContent: "center",
  },
  taskText: {
    flex: 1,
    minWidth: 0,
  },
  taskTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
  },
  taskMeta: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    marginTop: theme.spacing[1],
  },
  permissionBadge: {
    borderRadius: theme.borderRadius.sm,
    backgroundColor: theme.colors.surface3,
    paddingHorizontal: theme.spacing[2],
    paddingVertical: theme.spacing[1],
  },
  permissionBadgeText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
}));
