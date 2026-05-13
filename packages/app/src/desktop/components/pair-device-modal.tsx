import { AdaptiveModalSheet } from "@/components/adaptive-modal-sheet";
import { PairDeviceSection } from "@/desktop/components/pair-device-section";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import { useMemo } from "react";

export interface PairDeviceModalProps {
  visible: boolean;
  onClose: () => void;
  testID?: string;
}

export function PairDeviceModal({ visible, onClose, testID }: PairDeviceModalProps) {
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.host, [locale]);
  return (
    <AdaptiveModalSheet
      title={text.pairDeviceModalTitle}
      visible={visible}
      onClose={onClose}
      snapPoints={["82%", "94%"]}
      desktopMaxWidth={640}
      testID={testID}
    >
      <PairDeviceSection />
    </AdaptiveModalSheet>
  );
}
