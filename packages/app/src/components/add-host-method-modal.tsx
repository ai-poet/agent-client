import { useCallback, useMemo } from "react";
import { Pressable, Text, View } from "react-native";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { QrCode, Link2, ClipboardPaste } from "lucide-react-native";
import { AdaptiveModalSheet } from "./adaptive-modal-sheet";
import { isNative } from "@/constants/platform";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";

const styles = StyleSheet.create((theme) => ({
  option: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[4],
    padding: theme.spacing[4],
    borderRadius: theme.borderRadius.xl,
    backgroundColor: theme.colors.surface2,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  optionDisabled: {
    opacity: 0.55,
  },
  optionText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.normal,
  },
  optionSubtext: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.sm,
    marginTop: theme.spacing[1],
  },
  optionBody: {
    flex: 1,
  },
}));

export interface AddHostMethodModalProps {
  visible: boolean;
  onClose: () => void;
  onDirectConnection: () => void;
  onScanQr: () => void;
  onPasteLink: () => void;
}

export function AddHostMethodModal({
  visible,
  onClose,
  onDirectConnection,
  onScanQr,
  onPasteLink,
}: AddHostMethodModalProps) {
  const { theme } = useUnistyles();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.addHost, [locale]);

  const handleDirect = useCallback(() => {
    onDirectConnection();
  }, [onDirectConnection]);

  const handleScan = useCallback(() => {
    onScanQr();
  }, [onScanQr]);

  const handlePaste = useCallback(() => {
    onPasteLink();
  }, [onPasteLink]);

  return (
    <AdaptiveModalSheet
      title={text.title}
      visible={visible}
      onClose={onClose}
      testID="add-host-method-modal"
    >
      <Pressable
        style={styles.option}
        onPress={handleDirect}
        accessibilityRole="button"
        accessibilityLabel={text.directConnection}
        testID="add-host-method-direct"
      >
        <Link2 size={18} color={theme.colors.foreground} />
        <View style={styles.optionBody}>
          <Text style={styles.optionText}>{text.directConnection}</Text>
          <Text style={styles.optionSubtext}>{text.directConnectionHint}</Text>
        </View>
      </Pressable>

      {isNative ? (
        <Pressable
          style={[styles.option, styles.optionDisabled]}
          disabled
          onPress={handleScan}
          accessibilityRole="button"
          accessibilityLabel={text.scanQrComingSoon}
          testID="add-host-method-scan-qr-coming-soon"
        >
          <QrCode size={18} color={theme.colors.foreground} />
          <View style={styles.optionBody}>
            <Text style={styles.optionText}>{text.scanQrCode}</Text>
            <Text style={styles.optionSubtext}>{text.comingSoon}</Text>
          </View>
        </Pressable>
      ) : null}

      <Pressable
        style={styles.option}
        onPress={handlePaste}
        accessibilityRole="button"
        accessibilityLabel={text.pastePairingLink}
        testID="add-host-method-pair-link"
      >
        <ClipboardPaste size={18} color={theme.colors.foreground} />
        <View style={styles.optionBody}>
          <Text style={styles.optionText}>{text.pastePairingLink}</Text>
          <Text style={styles.optionSubtext}>{text.pastePairingLinkHint}</Text>
        </View>
      </Pressable>
    </AdaptiveModalSheet>
  );
}
