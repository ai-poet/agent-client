import { useCallback, useMemo, useRef } from "react";
import { ActivityIndicator, Keyboard, ScrollView, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import invariant from "tiny-invariant";
import { Composer } from "@/components/composer";
import { FileDropZone } from "@/components/file-drop-zone";
import { AgentStreamView } from "@/components/agent-stream-view";
import type { ImageAttachment } from "@/components/message-input";
import { useAppLocale } from "@/hooks/use-app-locale";
import { useAgentInputDraft } from "@/hooks/use-agent-input-draft";
import { useDraftAgentCreateFlow } from "@/hooks/use-draft-agent-create-flow";
import { useHostRuntimeClient, useHostRuntimeIsConnected } from "@/runtime/host-runtime";
import { buildWorkspaceDraftAgentConfig } from "@/screens/workspace/workspace-draft-agent-config";
import { shouldShowWorkspaceDraftSetupLoading } from "@/screens/workspace/workspace-draft-setup-loading";
import { buildDraftStoreKey } from "@/stores/draft-keys";
import type { Agent } from "@/stores/session-store";
import { useWorkspaceSetupStore } from "@/stores/workspace-setup-store";
import { useWorkspaceExecutionAuthority } from "@/stores/session-store-hooks";
import { buildWorkspaceTabPersistenceKey } from "@/stores/workspace-tabs-store";
import { encodeImages } from "@/utils/encode-images";
import { shouldAutoFocusWorkspaceDraftComposer } from "@/screens/workspace/workspace-draft-pane-focus";
import { getAppMessages } from "@/i18n/sub2api";
import type { AgentCapabilityFlags } from "@server/server/agent/agent-sdk-types";
import type { AgentSnapshotPayload } from "@server/shared/messages";
import { isWeb } from "@/constants/platform";

const EMPTY_PENDING_PERMISSIONS = new Map();
const DRAFT_CAPABILITIES: AgentCapabilityFlags = {
  supportsStreaming: true,
  supportsSessionPersistence: false,
  supportsDynamicModes: false,
  supportsMcpServers: false,
  supportsReasoningStream: false,
  supportsToolInvocations: false,
};

type WorkspaceDraftAgentTabProps = {
  serverId: string;
  workspaceId: string;
  tabId: string;
  draftId: string;
  isPaneFocused: boolean;
  onCreated: (snapshot: AgentSnapshotPayload) => void;
  onOpenWorkspaceFile: (input: { filePath: string }) => void;
};

export function WorkspaceDraftAgentTab({
  serverId,
  workspaceId,
  tabId,
  draftId,
  isPaneFocused,
  onCreated,
  onOpenWorkspaceFile,
}: WorkspaceDraftAgentTabProps) {
  const { theme } = useUnistyles();
  const insets = useSafeAreaInsets();
  const locale = useAppLocale();
  const messages = useMemo(() => getAppMessages(locale).draftAgent, [locale]);
  const client = useHostRuntimeClient(serverId);
  const isConnected = useHostRuntimeIsConnected(serverId);
  const workspaceAuthority = useWorkspaceExecutionAuthority(serverId, workspaceId);
  const workspaceExecutionAuthority = workspaceAuthority?.ok ? workspaceAuthority.authority : null;
  const workspaceDirectory = workspaceExecutionAuthority?.workspaceDirectory ?? null;
  const workspaceSetupKey = buildWorkspaceTabPersistenceKey({ serverId, workspaceId });
  const workspaceSetupSnapshot = useWorkspaceSetupStore((state) =>
    workspaceSetupKey ? (state.snapshots[workspaceSetupKey] ?? null) : null,
  );
  const shouldShowSetupLoading = shouldShowWorkspaceDraftSetupLoading({
    workspaceId,
    workspaceSetupSnapshot,
  });
  const addImagesRef = useRef<((images: ImageAttachment[]) => void) | null>(null);
  const draftStoreKey = useMemo(
    () =>
      buildDraftStoreKey({
        serverId,
        agentId: tabId,
        draftId,
      }),
    [draftId, serverId, tabId],
  );
  const draftInput = useAgentInputDraft({
    draftKey: draftStoreKey,
    initialCwd: workspaceDirectory ?? "",
    composer: {
      initialServerId: serverId,
      initialValues: workspaceDirectory ? { workingDir: workspaceDirectory } : undefined,
      isVisible: true,
      onlineServerIds: isConnected ? [serverId] : [],
      lockedWorkingDir: workspaceDirectory ?? undefined,
    },
  });
  const composerState = draftInput.composerState;
  if (!composerState) {
    throw new Error(messages.errors.composerStateRequired);
  }

  const {
    formErrorMessage,
    isSubmitting,
    optimisticStreamItems,
    draftAgent,
    handleCreateFromInput,
  } = useDraftAgentCreateFlow<Agent, AgentSnapshotPayload>({
    draftId,
    getPendingServerId: () => serverId,
    validateBeforeSubmit: ({ text }) => {
      if (!text.trim()) {
        return messages.errors.initialPromptRequired;
      }
      if (composerState.providerDefinitions.length === 0) {
        return messages.errors.noProviders;
      }
      if (!composerState.selectedProvider) {
        return messages.errors.selectModel;
      }
      if (composerState.isModelLoading) {
        return messages.errors.modelLoading;
      }
      if (!composerState.effectiveModelId) {
        return messages.errors.noModel;
      }
      if (!workspaceDirectory) {
        return messages.errors.workspaceDirectoryNotFound;
      }
      if (!client) {
        return messages.errors.hostNotConnected;
      }
      return null;
    },
    onBeforeSubmit: () => {
      void composerState.persistFormPreferences();
      if (isWeb) {
        (document.activeElement as HTMLElement | null)?.blur?.();
      }
      Keyboard.dismiss();
    },
    buildDraftAgent: (attempt) => {
      invariant(workspaceDirectory, messages.errors.workspaceDirectoryRequired);
      const now = attempt.timestamp;
      const model = composerState.effectiveModelId || null;
      const thinkingOptionId = composerState.effectiveThinkingOptionId || null;
      const modeId =
        composerState.modeOptions.length > 0 && composerState.selectedMode !== ""
          ? composerState.selectedMode
          : null;
      const provider = composerState.selectedProvider;
      if (!provider) {
        throw new Error(messages.errors.selectModel);
      }
      return {
        serverId,
        id: tabId,
        provider,
        status: "running",
        createdAt: now,
        updatedAt: now,
        lastUserMessageAt: now,
        lastActivityAt: now,
        capabilities: DRAFT_CAPABILITIES,
        currentModeId: modeId,
        availableModes: [],
        pendingPermissions: [],
        persistence: null,
        runtimeInfo: { provider, sessionId: null, model, modeId },
        title: messages.title,
        cwd: workspaceDirectory,
        model,
        features: composerState.statusControls.features,
        thinkingOptionId,
        labels: {},
      };
    },
    createRequest: async ({ attempt, text, images, attachments }) => {
      invariant(workspaceDirectory, messages.errors.workspaceDirectoryRequired);
      invariant(workspaceExecutionAuthority, messages.errors.workspaceAuthorityRequired);
      if (!client) {
        throw new Error(messages.errors.hostNotConnected);
      }

      const provider = composerState.selectedProvider;
      if (!provider) {
        throw new Error(messages.errors.selectModel);
      }
      const config = buildWorkspaceDraftAgentConfig({
        provider,
        cwd: workspaceDirectory,
        ...(composerState.modeOptions.length > 0 && composerState.selectedMode !== ""
          ? { modeId: composerState.selectedMode }
          : {}),
        model: composerState.effectiveModelId || undefined,
        thinkingOptionId: composerState.effectiveThinkingOptionId || undefined,
        featureValues: composerState.featureValues,
      });

      const imagesData = await encodeImages(images);
      const result = await client.createAgent({
        config,
        workspaceId: workspaceExecutionAuthority.workspaceId,
        ...(text ? { initialPrompt: text } : {}),
        clientMessageId: attempt.clientMessageId,
        ...(imagesData && imagesData.length > 0 ? { images: imagesData } : {}),
        ...(attachments && attachments.length > 0 ? { attachments } : {}),
      });

      return {
        agentId: result.id,
        result,
      };
    },
    onCreateSuccess: ({ result }) => {
      onCreated(result);
    },
  });

  const handleFilesDropped = useCallback((files: ImageAttachment[]) => {
    addImagesRef.current?.(files);
  }, []);

  const handleAddImagesCallback = useCallback((addImages: (images: ImageAttachment[]) => void) => {
    addImagesRef.current = addImages;
  }, []);

  const focusInputRef = useRef<(() => void) | null>(null);

  const handleFocusInputCallback = useCallback((focus: () => void) => {
    focusInputRef.current = focus;
  }, []);

  const handleProviderSelectWithFocus = useCallback(
    (provider: Parameters<typeof composerState.setProviderFromUser>[0]) => {
      composerState.setProviderFromUser(provider);
      focusInputRef.current?.();
    },
    [composerState],
  );

  const handleModeSelectWithFocus = useCallback(
    (modeId: string) => {
      composerState.setModeFromUser(modeId);
      focusInputRef.current?.();
    },
    [composerState],
  );

  const handleModelSelectWithFocus = useCallback(
    (modelId: string) => {
      composerState.setModelFromUser(modelId);
      focusInputRef.current?.();
    },
    [composerState],
  );

  const handleProviderAndModelSelectWithFocus = useCallback(
    (
      provider: Parameters<typeof composerState.setProviderAndModelFromUser>[0],
      modelId: string,
    ) => {
      composerState.setProviderAndModelFromUser(provider, modelId);
      focusInputRef.current?.();
    },
    [composerState],
  );

  const handleThinkingOptionSelectWithFocus = useCallback(
    (optionId: string) => {
      composerState.setThinkingOptionFromUser(optionId);
      focusInputRef.current?.();
    },
    [composerState],
  );

  const handleSetFeatureWithFocus = useCallback(
    (featureId: string, value: unknown) => {
      composerState.statusControls.onSetFeature?.(featureId, value);
      focusInputRef.current?.();
    },
    [composerState],
  );

  return (
    <FileDropZone onFilesDropped={handleFilesDropped}>
      <View style={styles.container}>
        <View style={styles.contentContainer}>
          {isSubmitting && draftAgent ? (
            <View style={styles.streamContainer}>
              <AgentStreamView
                agentId={tabId}
                serverId={serverId}
                agent={draftAgent}
                streamItems={optimisticStreamItems}
                pendingPermissions={EMPTY_PENDING_PERMISSIONS}
                onOpenWorkspaceFile={onOpenWorkspaceFile}
              />
            </View>
          ) : (
            <ScrollView
              style={styles.scrollView}
              contentContainerStyle={styles.configScrollContent}
            >
              <View style={styles.configSection}>
                {shouldShowSetupLoading ? (
                  <View style={styles.setupLoadingRow} testID="workspace-draft-setup-loading">
                    <ActivityIndicator size="small" color={theme.colors.foregroundMuted} />
                    <View style={styles.setupLoadingTextGroup}>
                      <Text style={styles.setupLoadingTitle}>{messages.setupLoadingTitle}</Text>
                      <Text style={styles.setupLoadingText}>{messages.setupLoadingBody}</Text>
                    </View>
                  </View>
                ) : null}
                {formErrorMessage ? (
                  <View style={styles.errorContainer}>
                    <Text style={styles.errorText}>{formErrorMessage}</Text>
                  </View>
                ) : null}
              </View>
            </ScrollView>
          )}
        </View>

        <View style={[styles.inputAreaWrapper, { paddingBottom: insets.bottom }]}>
          <Composer
            agentId={tabId}
            serverId={serverId}
            isPaneFocused={isPaneFocused}
            onSubmitMessage={handleCreateFromInput}
            isSubmitLoading={isSubmitting || shouldShowSetupLoading}
            blurOnSubmit={true}
            value={draftInput.text}
            onChangeText={draftInput.setText}
            attachments={draftInput.attachments}
            onChangeAttachments={draftInput.setAttachments}
            cwd={draftInput.cwd}
            clearDraft={draftInput.clear}
            autoFocus={shouldAutoFocusWorkspaceDraftComposer({ isPaneFocused, isSubmitting })}
            onAddImages={handleAddImagesCallback}
            onFocusInput={handleFocusInputCallback}
            commandDraftConfig={composerState.commandDraftConfig}
            statusControls={{
              ...composerState.statusControls,
              onSelectProvider: handleProviderSelectWithFocus,
              onSelectMode: handleModeSelectWithFocus,
              onSelectModel: handleModelSelectWithFocus,
              onSelectProviderAndModel: handleProviderAndModelSelectWithFocus,
              onSelectThinkingOption: handleThinkingOptionSelectWithFocus,
              onSetFeature: handleSetFeatureWithFocus,
              onDropdownClose: () => focusInputRef.current?.(),
              disabled: isSubmitting || shouldShowSetupLoading,
            }}
          />
        </View>
      </View>
    </FileDropZone>
  );
}

const styles = StyleSheet.create((theme) => ({
  container: {
    flex: 1,
    width: "100%",
    backgroundColor: theme.colors.surface0,
  },
  contentContainer: {
    flex: 1,
  },
  streamContainer: {
    flex: 1,
  },
  scrollView: {
    flex: 1,
  },
  configScrollContent: {
    paddingHorizontal: theme.spacing[4],
    paddingTop: theme.spacing[4],
    paddingBottom: theme.spacing[6],
  },
  configSection: {
    gap: theme.spacing[3],
  },
  setupLoadingRow: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: theme.spacing[3],
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.surface1,
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[3],
  },
  setupLoadingTextGroup: {
    flex: 1,
    gap: theme.spacing[1],
  },
  setupLoadingTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
  },
  setupLoadingText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    lineHeight: theme.fontSize.xs * 1.5,
  },
  inputAreaWrapper: {
    width: "100%",
    backgroundColor: theme.colors.surface0,
  },
  errorContainer: {
    marginTop: theme.spacing[2],
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[2],
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface2,
    borderWidth: 1,
    borderColor: theme.colors.destructive,
  },
  errorText: {
    color: theme.colors.destructive,
  },
}));
