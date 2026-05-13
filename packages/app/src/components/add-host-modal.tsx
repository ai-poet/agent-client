import { useCallback, useMemo, useRef, useState } from "react";
import { Alert, Text, TextInput, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { useIsCompactFormFactor } from "@/constants/layout";
import { Link2 } from "lucide-react-native";
import type { HostProfile } from "@/types/host-connection";
import { useHosts, useHostMutations } from "@/runtime/host-runtime";
import { normalizeHostPort } from "@/utils/daemon-endpoints";
import { DaemonConnectionTestError, connectToDaemon } from "@/utils/test-daemon-connection";
import { APP_NAME } from "@/config/branding";
import { AdaptiveModalSheet, AdaptiveTextInput } from "./adaptive-modal-sheet";
import { Button } from "@/components/ui/button";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";

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
  actions: {
    flexDirection: "row",
    gap: theme.spacing[3],
    marginTop: theme.spacing[2],
  },
  helper: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
  },
  error: {
    color: theme.colors.destructive,
    fontSize: theme.fontSize.sm,
  },
}));

function isHostPortOnly(raw: string): boolean {
  return !raw.includes("://") && !raw.includes("/");
}

function normalizeTransportMessage(message: string | null | undefined): string | null {
  if (!message) return null;
  const trimmed = message.trim();
  if (!trimmed) return null;
  return trimmed;
}

function formatTechnicalTransportDetails(details: Array<string | null>): string | null {
  const unique = Array.from(
    new Set(
      details
        .map((value) => normalizeTransportMessage(value))
        .filter((value): value is string => Boolean(value))
        .map((value) => value.trim())
        .filter((value) => value.length > 0),
    ),
  );

  if (unique.length === 0) return null;

  const allGeneric = unique.every((value) => {
    const lower = value.toLowerCase();
    return lower === "transport error" || lower === "transport closed";
  });

  if (allGeneric) {
    return unique[0];
  }

  return unique.join(" — ");
}

function buildConnectionFailureCopy(
  endpoint: string,
  error: unknown,
  text: ReturnType<typeof getSub2APIMessages>["settings"]["addHost"],
): { title: string; detail: string | null; raw: string | null } {
  const title = text.errors.failureTitle(endpoint);

  const raw = (() => {
    if (error instanceof DaemonConnectionTestError) {
      return (
        formatTechnicalTransportDetails([error.reason, error.lastError]) ??
        normalizeTransportMessage(error.message)
      );
    }
    if (error instanceof Error) {
      return normalizeTransportMessage(error.message);
    }
    return null;
  })();

  const rawLower = raw?.toLowerCase() ?? "";
  let detail: string | null = null;

  if (rawLower.includes("timed out")) {
    detail = text.errors.timeout;
  } else if (
    rawLower.includes("econnrefused") ||
    rawLower.includes("connection refused") ||
    rawLower.includes("err_connection_refused")
  ) {
    detail = text.errors.refused;
  } else if (rawLower.includes("enotfound") || rawLower.includes("not found")) {
    detail = text.errors.notFound;
  } else if (rawLower.includes("ehostunreach") || rawLower.includes("host is unreachable")) {
    detail = text.errors.unreachable;
  } else if (
    rawLower.includes("certificate") ||
    rawLower.includes("tls") ||
    rawLower.includes("ssl")
  ) {
    detail = text.errors.tls;
  } else if (raw) {
    detail = text.errors.unable;
  } else {
    detail = text.errors.unable;
  }

  return {
    title,
    detail,
    raw:
      raw && raw.toLowerCase() === "transport error"
        ? text.errors.noAdditionalDetails(raw)
        : raw,
  };
}

export interface AddHostModalProps {
  visible: boolean;
  onClose: () => void;
  onCancel?: () => void;
  onSaved?: (result: {
    profile: HostProfile;
    serverId: string;
    hostname: string | null;
    isNewHost: boolean;
  }) => void;
}

export function AddHostModal({ visible, onClose, onCancel, onSaved }: AddHostModalProps) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.addHost, [locale]);
  const daemons = useHosts();
  const { upsertDirectConnection } = useHostMutations();
  const isMobile = useIsCompactFormFactor();

  const hostInputRef = useRef<TextInput>(null);
  const endpointRawRef = useRef("");

  const [isSaving, setIsSaving] = useState(false);
  const [errorMessage, setErrorMessage] = useState("");

  const clearInput = useCallback(() => {
    endpointRawRef.current = "";
    hostInputRef.current?.clear();
  }, []);

  const handleClose = useCallback(() => {
    if (isSaving) return;
    clearInput();
    setErrorMessage("");
    onClose();
  }, [isSaving, clearInput, onClose]);

  const handleCancel = useCallback(() => {
    if (isSaving) return;
    clearInput();
    setErrorMessage("");
    (onCancel ?? onClose)();
  }, [isSaving, clearInput, onCancel, onClose]);

  const handleSave = useCallback(async () => {
    if (isSaving) return;

    const raw = endpointRawRef.current.trim();
    if (!raw) {
      setErrorMessage(text.errors.hostRequired);
      return;
    }
    if (!isHostPortOnly(raw)) {
      setErrorMessage(text.errors.hostPortOnly);
      return;
    }

    let endpoint: string;
    try {
      endpoint = normalizeHostPort(raw);
    } catch (error) {
      const message = error instanceof Error ? error.message : text.errors.invalidHostPort;
      setErrorMessage(message);
      return;
    }

    try {
      setIsSaving(true);
      setErrorMessage("");

      const { client, serverId, hostname } = await connectToDaemon({
        id: "probe",
        type: "directTcp",
        endpoint,
      });
      await client.close().catch(() => undefined);
      const isNewHost = !daemons.some((daemon) => daemon.serverId === serverId);
      const profile = await upsertDirectConnection({
        serverId,
        endpoint,
        label: hostname ?? undefined,
      });

      onSaved?.({ profile, serverId, hostname, isNewHost });
      handleClose();
    } catch (error) {
      const { title, detail, raw } = buildConnectionFailureCopy(endpoint, error, text);
      const combined =
        raw && detail && raw !== detail
          ? `${title}\n${detail}\n${text.errors.details}: ${raw}`
          : detail
            ? `${title}\n${detail}`
            : title;
      setErrorMessage(combined);
      if (!isMobile) {
        // Desktop/web: also surface it as a dialog for quick visibility.
        Alert.alert(text.connectionFailed, combined);
      }
    } finally {
      setIsSaving(false);
    }
  }, [daemons, handleClose, isMobile, isSaving, onSaved, text, upsertDirectConnection]);

  return (
    <AdaptiveModalSheet
      title={text.directTitle}
      visible={visible}
      onClose={handleClose}
      testID="add-host-modal"
    >
      <Text style={styles.helper}>{text.directHelper(APP_NAME)}</Text>

      <View style={styles.field}>
        <Text style={styles.label}>{text.host}</Text>
        <AdaptiveTextInput
          ref={hostInputRef}
          testID="direct-host-input"
          nativeID="direct-host-input"
          accessibilityLabel="direct-host-input"
          onChangeText={(next) => {
            endpointRawRef.current = next;
          }}
          placeholder={text.hostPlaceholder}
          placeholderTextColor={theme.colors.foregroundMuted}
          style={styles.input}
          autoCapitalize="none"
          autoCorrect={false}
          keyboardType="url"
          editable={!isSaving}
          returnKeyType="done"
          onSubmitEditing={() => void handleSave()}
        />
        {errorMessage ? <Text style={styles.error}>{errorMessage}</Text> : null}
      </View>

      <View style={styles.actions}>
        <Button style={{ flex: 1 }} variant="secondary" onPress={handleCancel} disabled={isSaving}>
          {text.cancel}
        </Button>
        <Button
          style={{ flex: 1 }}
          variant="default"
          onPress={() => void handleSave()}
          disabled={isSaving}
          leftIcon={<Link2 size={16} color={theme.colors.palette.white} />}
          testID="direct-host-submit"
        >
          {isSaving ? text.connecting : text.connect}
        </Button>
      </View>
    </AdaptiveModalSheet>
  );
}
