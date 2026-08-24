import { useCallback, useEffect, useRef, useState } from "react";
import { Pressable, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import * as Clipboard from "expo-clipboard";
import { Check, Copy } from "lucide-react-native";

const COPIED_FEEDBACK_MS = 1400;

interface FactRowProps {
  label: string;
  value: string;
  /** Render the value in the mono face — for paths, commands and versions. */
  mono?: boolean;
  /** Show a copy button. The copied state reverts on its own. */
  copyable?: boolean;
  copyAccessibilityLabel?: string;
  copiedAccessibilityLabel?: string;
}

/**
 * A label/value pair for reference information. Copyable values are what keeps a failed
 * automated action from becoming a dead end — the user can always run the command by hand.
 */
export function FactRow({
  label,
  value,
  mono = false,
  copyable = false,
  copyAccessibilityLabel,
  copiedAccessibilityLabel,
}: FactRowProps) {
  const { theme } = useUnistyles();
  const [copied, setCopied] = useState(false);
  const resetTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (resetTimer.current) {
        clearTimeout(resetTimer.current);
      }
    };
  }, []);

  const handleCopy = useCallback(() => {
    void Clipboard.setStringAsync(value)
      .then(() => {
        setCopied(true);
        if (resetTimer.current) {
          clearTimeout(resetTimer.current);
        }
        resetTimer.current = setTimeout(() => setCopied(false), COPIED_FEEDBACK_MS);
      })
      .catch((error) => {
        console.error("[FactRow] Failed to copy value", error);
      });
  }, [value]);

  return (
    <View style={styles.row}>
      <Text style={styles.label} numberOfLines={1}>
        {label}
      </Text>
      <View style={styles.valueGroup}>
        <Text style={[styles.value, mono && styles.valueMono]} numberOfLines={2} selectable>
          {value}
        </Text>
        {copyable ? (
          <Pressable
            onPress={handleCopy}
            accessibilityRole="button"
            accessibilityLabel={
              copied
                ? (copiedAccessibilityLabel ?? `${label} copied`)
                : (copyAccessibilityLabel ?? `Copy ${label}`)
            }
            style={styles.copyButton}
            hitSlop={6}
          >
            {copied ? (
              <Check size={theme.iconSize.xs} color={theme.colors.statusSuccess} />
            ) : (
              <Copy size={theme.iconSize.xs} color={theme.colors.foregroundMuted} />
            )}
          </Pressable>
        ) : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    justifyContent: "space-between",
    gap: theme.spacing[3],
    paddingVertical: theme.spacing[2],
  },
  label: {
    flexShrink: 0,
    fontSize: theme.fontSize.xs,
    color: theme.colors.foregroundMuted,
  },
  valueGroup: {
    flex: 1,
    flexDirection: "row",
    alignItems: "flex-start",
    justifyContent: "flex-end",
    gap: theme.spacing[2],
  },
  value: {
    flexShrink: 1,
    fontSize: theme.fontSize.xs,
    color: theme.colors.foreground,
    textAlign: "right",
  },
  valueMono: {
    fontFamily: "monospace",
  },
  copyButton: {
    flexDirection: "row",
    alignItems: "center",
    paddingTop: 1,
  },
}));
