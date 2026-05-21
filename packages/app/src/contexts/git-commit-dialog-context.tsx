import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Pressable, Text, View } from "react-native";
import { CheckSquare, GitCommitHorizontal, Square } from "lucide-react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { AdaptiveModalSheet, AdaptiveTextInput } from "@/components/adaptive-modal-sheet";
import { DiffStat } from "@/components/diff-stat";
import { Button } from "@/components/ui/button";
import { useAppLocale } from "@/hooks/use-app-locale";
import { getAppMessages } from "@/i18n/sub2api";

export type GitCommitDialogFile = {
  path: string;
  additions: number;
  deletions: number;
  isNew?: boolean;
  isDeleted?: boolean;
  status?: "ok" | "too_large" | "binary";
};

type GitCommitDialogInput = {
  serverId: string;
  cwd: string;
  files?: GitCommitDialogFile[];
  onCommit: (message: string, options?: { paths?: string[]; addAll?: boolean }) => Promise<void>;
};

type GitCommitDialogContextValue = {
  openCommitDialog: (input: GitCommitDialogInput) => void;
};

const GitCommitDialogContext = createContext<GitCommitDialogContextValue | null>(null);

export function useGitCommitDialog(): GitCommitDialogContextValue {
  const value = useContext(GitCommitDialogContext);
  if (!value) {
    throw new Error("useGitCommitDialog must be used within GitCommitDialogProvider");
  }
  return value;
}

export function GitCommitDialogProvider({ children }: { children: ReactNode }) {
  const { theme } = useUnistyles();
  const locale = useAppLocale();
  const text = useMemo(() => getAppMessages(locale).gitDiff, [locale]);
  const pendingInputRef = useRef<GitCommitDialogInput | null>(null);
  const [isOpen, setIsOpen] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(() => new Set());
  const files = pendingInputRef.current?.files ?? [];
  const hasFilePicker = files.length > 0;
  const selectedCount = hasFilePicker ? selectedPaths.size : 0;
  const allFilesSelected = hasFilePicker && selectedCount === files.length;

  const close = useCallback(() => {
    if (isSaving) {
      return;
    }
    setIsOpen(false);
    setMessage("");
    setError(null);
    setSelectedPaths(new Set());
    pendingInputRef.current = null;
  }, [isSaving]);

  const handleMessageChange = useCallback(
    (next: string) => {
      setMessage(next);
      if (error) {
        setError(null);
      }
    },
    [error],
  );

  const openCommitDialog = useCallback((input: GitCommitDialogInput) => {
    pendingInputRef.current = input;
    setMessage("");
    setError(null);
    setSelectedPaths(new Set(input.files?.map((file) => file.path) ?? []));
    setIsOpen(true);
  }, []);

  const togglePath = useCallback(
    (path: string) => {
      setSelectedPaths((current) => {
        const next = new Set(current);
        if (next.has(path)) {
          next.delete(path);
        } else {
          next.add(path);
        }
        return next;
      });
      if (error) {
        setError(null);
      }
    },
    [error],
  );

  const selectAllFiles = useCallback(() => {
    setSelectedPaths(new Set(files.map((file) => file.path)));
    if (error) {
      setError(null);
    }
  }, [error, files]);

  const clearFileSelection = useCallback(() => {
    setSelectedPaths(new Set());
    if (error) {
      setError(null);
    }
  }, [error]);

  const submit = useCallback(async () => {
    if (isSaving) {
      return;
    }
    const trimmed = message.trim();
    if (!trimmed) {
      setError(text.commitMessageRequired);
      return;
    }
    if (hasFilePicker && selectedPaths.size === 0) {
      setError(text.commitFilesRequired);
      return;
    }

    const input = pendingInputRef.current;
    if (!input) {
      setIsOpen(false);
      return;
    }

    setIsSaving(true);
    setError(null);
    try {
      const selectedFilePaths = hasFilePicker
        ? files.filter((file) => selectedPaths.has(file.path)).map((file) => file.path)
        : undefined;
      const commitOptions = selectedFilePaths
        ? { paths: selectedFilePaths, addAll: false }
        : undefined;
      await input.onCommit(trimmed, commitOptions);
      setIsOpen(false);
      setMessage("");
      setSelectedPaths(new Set());
      pendingInputRef.current = null;
    } catch (err) {
      setError(err instanceof Error ? err.message : text.failedCommit);
    } finally {
      setIsSaving(false);
    }
  }, [
    files,
    hasFilePicker,
    isSaving,
    message,
    selectedPaths,
    text.commitFilesRequired,
    text.commitMessageRequired,
    text.failedCommit,
  ]);

  const value = useMemo(() => ({ openCommitDialog }), [openCommitDialog]);

  return (
    <GitCommitDialogContext.Provider value={value}>
      {children}
      <AdaptiveModalSheet
        title={text.commitDialogTitle}
        visible={isOpen}
        onClose={close}
        testID="git-commit-dialog"
        desktopMaxWidth={560}
      >
        <View style={styles.field}>
          <Text style={styles.label}>{text.commitMessageLabel}</Text>
          <AdaptiveTextInput
            testID="git-commit-message-input"
            value={message}
            onChangeText={handleMessageChange}
            placeholder={text.commitMessagePlaceholder}
            placeholderTextColor={theme.colors.foregroundMuted}
            style={styles.input}
            editable={!isSaving}
            autoCapitalize="sentences"
            autoCorrect
            returnKeyType="done"
            onSubmitEditing={() => void submit()}
          />
          {error ? <Text style={styles.error}>{error}</Text> : null}
        </View>
        {hasFilePicker ? (
          <View style={styles.filePicker}>
            <View style={styles.filePickerHeader}>
              <View style={styles.filePickerTitleGroup}>
                <Text style={styles.label}>{text.commitFilesLabel}</Text>
                <Text style={styles.filePickerCount}>
                  {text.commitFilesSelected(selectedCount, files.length)}
                </Text>
              </View>
              <Button
                variant="ghost"
                size="sm"
                onPress={allFilesSelected ? clearFileSelection : selectAllFiles}
                disabled={isSaving}
                testID="git-commit-toggle-all-files"
              >
                {allFilesSelected ? text.clearCommitFiles : text.selectAllCommitFiles}
              </Button>
            </View>
            <View style={styles.fileList}>
              {files.map((file) => {
                const isSelected = selectedPaths.has(file.path);
                const fileName = file.path.split("/").pop() || file.path;
                const fileDir = file.path.includes("/")
                  ? file.path.slice(0, file.path.lastIndexOf("/"))
                  : "";
                return (
                  <Pressable
                    key={file.path}
                    accessibilityRole="checkbox"
                    accessibilityState={{ checked: isSelected, disabled: isSaving }}
                    onPress={() => togglePath(file.path)}
                    disabled={isSaving}
                    style={({ hovered, pressed }) => [
                      styles.fileRow,
                      isSelected && styles.fileRowSelected,
                      (hovered || pressed) && !isSaving && styles.fileRowHovered,
                    ]}
                    testID={`git-commit-file-${file.path}`}
                  >
                    {isSelected ? (
                      <CheckSquare size={18} color={theme.colors.accent} />
                    ) : (
                      <Square size={18} color={theme.colors.foregroundMuted} />
                    )}
                    <View style={styles.fileTextGroup}>
                      <View style={styles.fileNameRow}>
                        <Text style={styles.fileName} numberOfLines={1}>
                          {fileName}
                        </Text>
                        {file.isNew ? (
                          <View style={styles.fileBadge}>
                            <Text style={styles.fileBadgeText}>{text.newFileBadge}</Text>
                          </View>
                        ) : null}
                        {file.isDeleted ? (
                          <View style={[styles.fileBadge, styles.deletedFileBadge]}>
                            <Text style={[styles.fileBadgeText, styles.deletedFileBadgeText]}>
                              {text.deletedFileBadge}
                            </Text>
                          </View>
                        ) : null}
                        {file.status === "binary" ? (
                          <View style={styles.fileBadge}>
                            <Text style={styles.fileBadgeText}>{text.binaryFileBadge}</Text>
                          </View>
                        ) : null}
                      </View>
                      {fileDir ? (
                        <Text style={styles.fileDir} numberOfLines={1}>
                          {fileDir}
                        </Text>
                      ) : null}
                    </View>
                    <DiffStat additions={file.additions} deletions={file.deletions} />
                  </Pressable>
                );
              })}
            </View>
          </View>
        ) : null}
        <View style={styles.actions}>
          <Button
            style={styles.actionButton}
            variant="secondary"
            onPress={close}
            disabled={isSaving}
          >
            {text.cancelCommit}
          </Button>
          <Button
            style={styles.actionButton}
            variant="default"
            onPress={() => void submit()}
            disabled={isSaving}
            leftIcon={<GitCommitHorizontal size={16} color={theme.colors.palette.white} />}
            testID="git-commit-submit"
          >
            {isSaving ? text.committing : text.commitAction}
          </Button>
        </View>
      </AdaptiveModalSheet>
    </GitCommitDialogContext.Provider>
  );
}

const styles = StyleSheet.create((theme) => ({
  field: {
    gap: theme.spacing[2],
  },
  label: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
  },
  input: {
    backgroundColor: theme.colors.surface2,
    borderRadius: theme.borderRadius.lg,
    paddingHorizontal: theme.spacing[4],
    paddingVertical: theme.spacing[3],
    color: theme.colors.foreground,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  error: {
    color: theme.colors.destructive,
    fontSize: theme.fontSize.sm,
  },
  filePicker: {
    gap: theme.spacing[3],
  },
  filePickerHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[3],
  },
  filePickerTitleGroup: {
    flex: 1,
    minWidth: 0,
    gap: theme.spacing[1],
  },
  filePickerCount: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  fileList: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.lg,
    overflow: "hidden",
  },
  fileRow: {
    minHeight: 52,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[3],
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[2],
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
    backgroundColor: theme.colors.surface1,
  },
  fileRowSelected: {
    backgroundColor: theme.colors.surface2,
  },
  fileRowHovered: {
    backgroundColor: theme.colors.surface3,
  },
  fileTextGroup: {
    flex: 1,
    minWidth: 0,
    gap: 2,
  },
  fileNameRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
    minWidth: 0,
  },
  fileName: {
    flexShrink: 1,
    minWidth: 0,
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
  },
  fileDir: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  fileBadge: {
    flexShrink: 0,
    borderRadius: theme.borderRadius.md,
    paddingHorizontal: theme.spacing[2],
    paddingVertical: 2,
    backgroundColor: theme.colors.surface3,
  },
  fileBadgeText: {
    color: theme.colors.diffAddition,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
  },
  deletedFileBadge: {
    backgroundColor: theme.colors.surface3,
  },
  deletedFileBadgeText: {
    color: theme.colors.diffDeletion,
  },
  actions: {
    flexDirection: "row",
    gap: theme.spacing[3],
    marginTop: theme.spacing[4],
  },
  actionButton: {
    flex: 1,
  },
}));
