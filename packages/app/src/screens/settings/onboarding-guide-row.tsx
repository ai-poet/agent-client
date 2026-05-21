import React from "react";
import { BookOpen } from "lucide-react-native";
import { Text, View } from "react-native";
import { Button } from "@/components/ui/button";
import { settingsStyles } from "@/styles/settings";

interface OnboardingGuideRowProps {
  title: string;
  hint: string;
  actionLabel: string;
  accessibilityLabel: string;
  onReplay: () => void;
}

export function OnboardingGuideRow({
  title,
  hint,
  actionLabel,
  accessibilityLabel,
  onReplay,
}: OnboardingGuideRowProps) {
  return (
    <View style={[settingsStyles.row, settingsStyles.rowBorder]}>
      <View style={settingsStyles.rowContent}>
        <Text style={settingsStyles.rowTitle}>{title}</Text>
        <Text style={settingsStyles.rowHint}>{hint}</Text>
      </View>
      <Button
        variant="secondary"
        size="sm"
        leftIcon={BookOpen}
        onPress={onReplay}
        accessibilityLabel={accessibilityLabel}
        testID="settings-replay-onboarding-guide"
      >
        {actionLabel}
      </Button>
    </View>
  );
}
