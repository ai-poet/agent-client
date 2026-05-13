import { useCallback } from "react";
import { Pressable, Text, View } from "react-native";
import { useRouter } from "expo-router";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { BackHeader } from "@/components/headers/back-header";

const styles = StyleSheet.create((theme) => ({
  container: {
    flex: 1,
    backgroundColor: theme.colors.surface0,
  },
  body: {
    flex: 1,
    paddingHorizontal: theme.spacing[6],
  },
  card: {
    marginTop: theme.spacing[6],
    padding: theme.spacing[6],
    borderRadius: theme.borderRadius.xl,
    backgroundColor: theme.colors.surface2,
    gap: theme.spacing[4],
  },
  title: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.lg,
    fontWeight: theme.fontWeight.semibold,
  },
  bodyText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.base,
    lineHeight: theme.fontSize.base * 1.45,
  },
  button: {
    alignSelf: "flex-start",
    paddingHorizontal: theme.spacing[6],
    paddingVertical: theme.spacing[3],
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.palette.blue[500],
  },
  buttonText: {
    color: theme.colors.palette.white,
    fontWeight: theme.fontWeight.semibold,
  },
}));

export default function PairScanScreen() {
  const { theme } = useUnistyles();
  const insets = useSafeAreaInsets();
  const router = useRouter();

  const closeToSource = useCallback(() => {
    try {
      router.back();
    } catch {
      router.replace("/" as any);
    }
  }, [router]);

  return (
    <View style={styles.container}>
      <BackHeader title="Scan QR" onBack={closeToSource} />

      <View style={[styles.body, { paddingBottom: insets.bottom + theme.spacing[6] }]}>
        <View style={styles.card}>
          <Text style={styles.title}>Coming soon</Text>
          <Text style={styles.bodyText}>
            QR pairing is temporarily unavailable. Paste a pairing link or use direct connection for
            now.
          </Text>
          <Pressable style={styles.button} onPress={closeToSource}>
            <Text style={styles.buttonText}>Back</Text>
          </Pressable>
        </View>
      </View>
    </View>
  );
}
