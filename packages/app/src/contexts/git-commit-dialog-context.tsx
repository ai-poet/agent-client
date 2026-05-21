import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Text, View } from "react-native";
import { GitCommitHorizontal } from "lucide-react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { AdaptiveModalSheet, AdaptiveTextInput } from "@/components/adaptive-modal-sheet";
import { Button } from "@/components/ui/button";
import { useAppLocale } from "@/hooks/use-app-locale";
import { getAppMessages } from "@/i18n/sub2api";

type GitCommitDialogInput = {
  serverId: string;
  cwd: string;
  onCommit: (message: string) => Promise<void>;
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

  const close = useCallback(() => {
    if (isSaving) {
      return;
    }
    setIsOpen(false);
    setMessage("");
    setError(null);
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
    setIsOpen(true);
  }, []);

  const submit = useCallback(async () => {
    if (isSaving) {
      return;
    }
    const trimmed = message.trim();
    if (!trimmed) {
      setError(text.commitMessageRequired);
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
      await input.onCommit(trimmed);
      setIsOpen(false);
      setMessage("");
      pendingInputRef.current = null;
    } catch (err) {
      setError(err instanceof Error ? err.message : text.failedCommit);
    } finally {
      setIsSaving(false);
    }
  }, [isSaving, message, text.commitMessageRequired, text.failedCommit]);

  const value = useMemo(() => ({ openCommitDialog }), [openCommitDialog]);

  return (
    <GitCommitDialogContext.Provider value={value}>
      {children}
      <AdaptiveModalSheet
        title={text.commitDialogTitle}
        visible={isOpen}
        onClose={close}
        testID="git-commit-dialog"
        desktopMaxWidth={460}
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
  actions: {
    flexDirection: "row",
    gap: theme.spacing[3],
    marginTop: theme.spacing[4],
  },
  actionButton: {
    flex: 1,
  },
}));
