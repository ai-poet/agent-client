import { useMemo } from "react";
import { Pressable, Text, View } from "react-native";
import { useRouter } from "expo-router";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { StyleSheet, useUnistyles } from "react-native-unistyles";
import { ArrowLeft, Settings } from "lucide-react-native";
import { Button } from "@/components/ui/button";
import { StandaloneAgentConversation } from "@/panels/agent-panel";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { buildHostSimpleRoute, buildSettingsRoute } from "@/utils/host-routes";

export function SimpleAgentScreen({
  serverId,
  agentId,
}: {
  serverId: string;
  agentId: string;
}) {
  const { theme } = useUnistyles();
  const insets = useSafeAreaInsets();
  const router = useRouter();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).simpleMode, [locale]);

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
        <Pressable
          style={({ pressed }) => [styles.backButton, pressed ? styles.pressed : null]}
          onPress={() => router.replace(buildHostSimpleRoute(serverId))}
        >
          <ArrowLeft size={18} color={theme.colors.foreground} />
          <Text style={styles.backText}>{text.tasks}</Text>
        </Pressable>
        <Button
          size="sm"
          variant="ghost"
          leftIcon={Settings}
          onPress={() => router.push(buildSettingsRoute())}
        >
          {text.settings}
        </Button>
      </View>
      <View style={styles.body}>
        <StandaloneAgentConversation serverId={serverId} agentId={agentId} />
      </View>
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  root: {
    flex: 1,
    backgroundColor: theme.colors.surface0,
  },
  header: {
    minHeight: 64,
    paddingHorizontal: theme.spacing[6],
    paddingBottom: theme.spacing[3],
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
    backgroundColor: theme.colors.surface0,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: theme.spacing[3],
  },
  backButton: {
    minHeight: 40,
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing[2],
  },
  backText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.medium,
  },
  pressed: {
    opacity: 0.82,
  },
  body: {
    flex: 1,
  },
}));
