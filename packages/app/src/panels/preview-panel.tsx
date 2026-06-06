import { Globe } from "lucide-react-native";
import invariant from "tiny-invariant";
import { BrowserPreviewPane } from "@/components/browser-preview-pane";
import { usePaneContext } from "@/panels/pane-context";
import type { PanelRegistration } from "@/panels/panel-registry";
import { useWorkspace } from "@/stores/session-store-hooks";

function usePreviewPanelDescriptor(target: { kind: "preview"; scriptName: string }) {
  return {
    label: target.scriptName,
    subtitle: "Preview",
    titleState: "ready" as const,
    icon: Globe,
    statusBucket: null,
  };
}

function PreviewPanel() {
  const { serverId, workspaceId, target } = usePaneContext();
  invariant(target.kind === "preview", "PreviewPanel requires preview target");
  const workspace = useWorkspace(serverId, workspaceId);

  return (
    <BrowserPreviewPane
      serverId={serverId}
      scriptName={target.scriptName}
      scripts={workspace?.scripts ?? []}
    />
  );
}

export const previewPanelRegistration: PanelRegistration<"preview"> = {
  kind: "preview",
  component: PreviewPanel,
  useDescriptor: usePreviewPanelDescriptor,
};
