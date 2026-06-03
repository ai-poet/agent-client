/**
 * @vitest-environment jsdom
 */
import React, { type ReactElement } from "react";
import { act } from "@testing-library/react";
import type { WorkspaceScriptPayload } from "@server/shared/messages";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot, type Root } from "react-dom/client";
import { BrowserPreviewPane } from "@/components/browser-preview-pane.web";

const { activeConnectionState, openExternalUrlMock, theme } = vi.hoisted(() => ({
  activeConnectionState: {
    activeConnection: null as null | {
      type: "directTcp" | "directSocket" | "directPipe" | "relay";
      endpoint: string;
      display: string;
    },
  },
  openExternalUrlMock: vi.fn(async () => {}),
  theme: {
    spacing: { 1: 4, 2: 8, 3: 12, 6: 24 },
    borderWidth: { 1: 1 },
    fontSize: { xs: 11, sm: 13, lg: 18 },
    fontWeight: { medium: "600" },
    colors: {
      border: "#ddd",
      foreground: "#111",
      foregroundMuted: "#666",
      surface0: "#fff",
      surface1: "#fafafa",
      surface2: "#f4f4f5",
    },
  },
}));

vi.mock("react-native-unistyles", () => ({
  StyleSheet: {
    create: (factory: unknown) => (typeof factory === "function" ? factory(theme) : factory),
  },
  useUnistyles: () => ({ theme }),
}));

vi.mock("@/hooks/use-app-locale", () => ({
  useAppLocale: () => "en",
}));

vi.mock("@/runtime/host-runtime", () => ({
  useHostRuntimeSnapshot: () => ({ activeConnection: activeConnectionState.activeConnection }),
}));

vi.mock("@/utils/open-external-url", () => ({
  openExternalUrl: openExternalUrlMock,
}));

vi.mock("lucide-react-native", () => {
  const createIcon = (name: string) => (props: Record<string, unknown>) =>
    React.createElement("span", {
      "data-icon": name,
      "data-color": props.color,
      "data-size": props.size,
    });
  return {
    ExternalLink: createIcon("ExternalLink"),
    RotateCw: createIcon("RotateCw"),
  };
});

vi.mock("react-native", async () => {
  const actual = await vi.importActual<Record<string, unknown>>("react-native");
  return actual;
});

function script(
  input: Partial<WorkspaceScriptPayload> & Pick<WorkspaceScriptPayload, "scriptName">,
): WorkspaceScriptPayload {
  return {
    scriptName: input.scriptName,
    type: input.type ?? "service",
    hostname: input.hostname ?? input.scriptName,
    port: input.port ?? null,
    proxyUrl: input.proxyUrl ?? null,
    lifecycle: input.lifecycle ?? "running",
    health: input.health ?? null,
    exitCode: input.exitCode ?? null,
    terminalId: input.terminalId ?? null,
  };
}

let root: Root | null = null;
let container: HTMLElement | null = null;

function renderPreview(element: ReactElement): void {
  act(() => {
    root?.render(element);
  });
}

function getIframe(): HTMLIFrameElement | null {
  return container?.querySelector("iframe") ?? null;
}

describe("BrowserPreviewPane", () => {
  beforeEach(() => {
    vi.stubGlobal("React", React);
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    activeConnectionState.activeConnection = {
      type: "directTcp",
      endpoint: "localhost:6767",
      display: "localhost:6767",
    };
    openExternalUrlMock.mockClear();
    document.body.innerHTML = "";
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    if (root) {
      act(() => {
        root?.unmount();
      });
    }
    root = null;
    container?.remove();
    container = null;
    vi.unstubAllGlobals();
  });

  it("renders an iframe from the resolved service URL", () => {
    renderPreview(
      <BrowserPreviewPane
        serverId="test-server"
        scriptName="web"
        scripts={[
          script({
            scriptName: "web",
            port: 3000,
            proxyUrl: "http://web.paseo.localhost:6767",
          }),
        ]}
      />,
    );

    expect(container?.querySelector('[data-testid="browser-preview-pane"]')).not.toBeNull();
    expect(getIframe()?.getAttribute("src")).toBe("http://web.paseo.localhost:6767");
    expect(container?.textContent).toContain("web.paseo.localhost:6767");
  });

  it("reloads by remounting the iframe", async () => {
    renderPreview(
      <BrowserPreviewPane
        serverId="test-server"
        scriptName="web"
        scripts={[
          script({
            scriptName: "web",
            port: 3000,
            proxyUrl: "http://web.paseo.localhost:6767",
          }),
        ]}
      />,
    );

    const before = getIframe();
    expect(before).not.toBeNull();

    await act(async () => {
      container?.querySelector<HTMLElement>('[data-testid="browser-preview-reload"]')?.click();
    });

    const after = getIframe();
    expect(after).not.toBeNull();
    expect(after).not.toBe(before);
    expect(after?.getAttribute("src")).toBe("http://web.paseo.localhost:6767");
  });

  it("opens the resolved URL externally", async () => {
    renderPreview(
      <BrowserPreviewPane
        serverId="test-server"
        scriptName="web"
        scripts={[
          script({
            scriptName: "web",
            port: 3000,
            proxyUrl: "http://web.paseo.localhost:6767",
          }),
        ]}
      />,
    );

    await act(async () => {
      container
        ?.querySelector<HTMLElement>('[data-testid="browser-preview-open-external"]')
        ?.click();
    });

    expect(openExternalUrlMock).toHaveBeenCalledWith("http://web.paseo.localhost:6767");
  });

  it("does not render an iframe for stopped or missing services", () => {
    renderPreview(
      <BrowserPreviewPane
        serverId="test-server"
        scriptName="web"
        scripts={[
          script({
            scriptName: "web",
            lifecycle: "stopped",
            port: 3000,
            proxyUrl: "http://web.paseo.localhost:6767",
          }),
        ]}
      />,
    );

    expect(
      container?.querySelector('[data-testid="browser-preview-pane-unavailable"]'),
    ).not.toBeNull();
    expect(getIframe()).toBeNull();

    renderPreview(
      <BrowserPreviewPane
        serverId="test-server"
        scriptName="api"
        scripts={[
          script({
            scriptName: "web",
            port: 3000,
            proxyUrl: "http://web.paseo.localhost:6767",
          }),
        ]}
      />,
    );

    expect(
      container?.querySelector('[data-testid="browser-preview-pane-unavailable"]'),
    ).not.toBeNull();
    expect(getIframe()).toBeNull();
  });
});
