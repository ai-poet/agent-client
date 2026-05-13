/**
 * @vitest-environment jsdom
 */
import React from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { mocks } = vi.hoisted(() => ({
  mocks: {
    getAccessToken: vi.fn(),
    loadProviders: vi.fn(),
    handleGitHubLogin: vi.fn(),
    logout: vi.fn(),
    activeClaudeApiKey: "sk-claude",
    activeCodexApiKey: "sk-codex",
  },
}));

const { theme } = vi.hoisted(() => ({
  theme: {
    colors: {
      foreground: "#111",
      foregroundMuted: "#666",
      border: "#ddd",
      surface0: "#fff",
      surface1: "#fafafa",
      surface2: "#f0f0f0",
      accent: "#2563eb",
      accentForeground: "#fff",
      destructive: "#dc2626",
      palette: {
        green: { 400: "#22c55e" },
        white: "#fff",
      },
    },
    spacing: { 1: 4, 2: 8, 3: 12, 4: 16, 6: 24 },
    borderRadius: { md: 8, lg: 12, xl: 16 },
    fontSize: { xs: 12, sm: 14, base: 16, lg: 20 },
    fontWeight: { normal: "400", medium: "600" },
    opacity: { 50: 0.5 },
    iconSize: { sm: 16, md: 20, lg: 24 },
  },
}));

vi.mock("react-native-unistyles", () => ({
  useUnistyles: () => ({ theme, rt: { breakpoint: "lg" } }),
  StyleSheet: {
    create: (factory: any) => factory(theme),
    absoluteFillObject: {},
  },
}));

vi.mock("lucide-react-native", () => ({
  Cloud: (props: Record<string, unknown>) => React.createElement("span", props, "cloud"),
}));

vi.mock("react-native", () => {
  const normalizeStyle = (style: unknown) => {
    if (Array.isArray(style)) {
      return Object.assign(
        {},
        ...style.filter(
          (item) => typeof item === "object" && item !== null && !Array.isArray(item),
        ),
      );
    }
    return typeof style === "object" && style !== null ? style : undefined;
  };

  const mapProps = (props: Record<string, unknown>) => {
    const { testID, children, onPress, style, numberOfLines, ...rest } = props;
    return {
      ...rest,
      ...(normalizeStyle(style) ? { style: normalizeStyle(style) } : {}),
      ...(typeof testID === "string" ? { "data-testid": testID } : {}),
      ...(typeof onPress === "function" ? { onClick: onPress } : {}),
      children,
    };
  };

  return {
    Alert: { alert: vi.fn() },
    View: (props: Record<string, unknown>) => React.createElement("div", mapProps(props)),
    Text: (props: Record<string, unknown>) => React.createElement("span", mapProps(props)),
    Pressable: (props: Record<string, unknown>) => {
      const children =
        typeof props.children === "function" ? props.children({ pressed: false }) : props.children;
      return React.createElement("button", mapProps({ ...props, children }));
    },
  };
});

vi.mock("@/constants/layout", () => ({
  useIsCompactFormFactor: () => false,
}));

vi.mock("@/config/branding", () => ({
  APP_NAME: "Paseo",
  CLOUD_NAME: "Paseo Cloud",
  DESKTOP_DEFAULT_KEY_NAME: "Paseo Desktop",
}));

vi.mock("expo-constants", () => ({
  default: {
    expoConfig: {
      extra: { brand: { appName: "Paseo", cloudName: "Paseo Cloud" } },
    },
  },
}));

vi.mock("expo-router", () => ({
  useRouter: () => ({ replace: vi.fn() }),
}));

vi.mock("@/hooks/use-settings", () => ({
  useAppSettings: () => ({
    settings: {
      accessMode: "byok",
      language: "en",
    },
  }),
}));

vi.mock("@/hooks/use-sub2api-auth", () => ({
  useSub2APIAuth: () => ({
    getAccessToken: mocks.getAccessToken,
  }),
}));

vi.mock("@/hooks/use-sub2api-login-flow", () => ({
  useSub2APILoginFlow: () => ({
    endpoint: "https://api.example.com",
    canStartLogin: true,
    isLoggedIn: true,
    auth: { endpoint: "https://api.example.com" },
    handleGitHubLogin: mocks.handleGitHubLogin,
    logout: mocks.logout,
  }),
}));

vi.mock("@/hooks/use-sub2api-api", () => ({
  useSub2APIMe: () => ({
    data: { balance: 12.34, username: "alice" },
    isPending: false,
    isFetching: false,
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  }),
  useSub2APIUsageStats: () => ({
    data: { total_cost: 1.23, total_actual_cost: 1.23, total_requests: 5 },
    refetch: vi.fn(),
  }),
  useSub2APIKeys: () => ({
    data: {
      items: [
        {
          id: 11,
          key: "sk-claude",
          name: "Claude Cloud Key",
          group_id: 111,
          quota: 20,
          quota_used: 4,
          rate_limit_5h: 0,
          rate_limit_1d: 0,
          rate_limit_7d: 0,
          usage_5h: 0.4,
          usage_1d: 0.9,
          usage_7d: 2,
          group: { id: 111, name: "Anthropic Group", platform: "anthropic" },
        },
        {
          id: 22,
          key: "sk-codex",
          name: "Codex Cloud Key",
          group_id: 222,
          quota: 0,
          quota_used: 8,
          rate_limit_5h: 0,
          rate_limit_1d: 0,
          rate_limit_7d: 0,
          usage_5h: 0.8,
          usage_1d: 1.5,
          usage_7d: 5,
          group: { id: 222, name: "OpenAI Group", platform: "openai" },
        },
      ],
    },
  }),
}));

vi.mock("@/screens/settings/desktop-providers-context", () => ({
  useDesktopProvidersStore: () => ({
    loadProviders: mocks.loadProviders,
    activeClaudeProvider: {
      name: "Paseo Claude",
      endpoint: "https://api.example.com",
      apiKey: mocks.activeClaudeApiKey,
    },
    activeCodexProvider: {
      name: "Paseo Codex",
      endpoint: "https://api.example.com",
      apiKey: mocks.activeCodexApiKey,
    },
  }),
}));

vi.mock("@/screens/settings/settings-section", () => ({
  SettingsSection: ({ title, children }: any) =>
    React.createElement("section", null, React.createElement("h2", null, title), children),
}));

vi.mock("@/screens/settings/paseo-cloud-api-keys-section", () => ({
  PaseoCloudApiKeysSection: () => React.createElement("div", null, "API keys section content"),
}));

vi.mock("@/screens/settings/paseo-cloud-routing-section", () => ({
  PaseoCloudRoutingSection: () => React.createElement("div", null, "Routing section content"),
}));

vi.mock("@/screens/settings/sub2api-models-section", () => ({
  Sub2APIModelsSection: () => React.createElement("div", null, "Model catalog section content"),
}));

vi.mock("@/screens/settings/paseo-cloud-referral-section", () => ({
  PaseoCloudReferralSection: () => React.createElement("div", null, "Referral section content"),
}));

vi.mock("@/screens/settings/paseo-cloud-usage-section", () => ({
  PaseoCloudUsageSection: () => React.createElement("div", null, "Usage section content"),
}));

vi.mock("@/screens/settings/paseo-cloud-model-status-section", () => ({
  PaseoCloudModelStatusSection: () =>
    React.createElement("div", null, "Model status section content"),
}));

vi.mock("@/screens/settings/sub2api-pay-modal", () => ({
  Sub2APIPayModal: () => null,
}));

describe("PaseoCloudPanel", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    mocks.getAccessToken.mockReset();
    mocks.loadProviders.mockReset();
    mocks.handleGitHubLogin.mockReset();
    mocks.logout.mockReset();
    mocks.activeClaudeApiKey = "sk-claude";
    mocks.activeCodexApiKey = "sk-codex";
  });

  it("switches between internal Paseo Cloud sections from the left menu", async () => {
    const { PaseoCloudPanel } = await import("./paseo-cloud-panel");

    render(<PaseoCloudPanel />);

    expect(screen.queryByText("Signed in as alice")).not.toBeNull();
    expect(screen.queryByText("Current routes")).not.toBeNull();
    expect(screen.queryByText(/Anthropic Group/)).not.toBeNull();
    expect(screen.queryByText(/OpenAI Group/)).not.toBeNull();

    fireEvent.click(screen.getByTestId("paseo-cloud-section-keys"));
    expect(screen.queryByText("API keys section content")).not.toBeNull();

    fireEvent.click(screen.getByTestId("paseo-cloud-section-routing"));
    expect(screen.queryByText("Routing section content")).not.toBeNull();

    fireEvent.click(screen.getByTestId("paseo-cloud-section-catalog"));
    expect(screen.queryByText("Model catalog section content")).not.toBeNull();

    fireEvent.click(screen.getByTestId("paseo-cloud-section-usage"));
    expect(screen.queryByText("Usage section content")).not.toBeNull();

    fireEvent.click(screen.getByTestId("paseo-cloud-section-status"));
    expect(screen.queryByText("Model status section content")).not.toBeNull();
  });

  it("supports deep-linked initial sections and overview quick actions", async () => {
    const { PaseoCloudPanel } = await import("./paseo-cloud-panel");

    render(<PaseoCloudPanel initialSection="keys" />);

    expect(screen.queryByText("API keys section content")).not.toBeNull();

    fireEvent.click(screen.getByTestId("paseo-cloud-section-overview"));
    fireEvent.click(screen.getByTestId("paseo-cloud-overview-open-routing"));
    expect(screen.queryByText("Routing section content")).not.toBeNull();

    fireEvent.click(screen.getByTestId("paseo-cloud-section-overview"));
    fireEvent.click(screen.getByTestId("paseo-cloud-overview-open-catalog"));
    expect(screen.queryByText("Model catalog section content")).not.toBeNull();
  });

  it("offers repair actions when the device route key is not in the signed-in account", async () => {
    mocks.activeClaudeApiKey = "sk-unlinked";
    const { PaseoCloudPanel } = await import("./paseo-cloud-panel");

    render(<PaseoCloudPanel />);

    expect(
      screen.queryByText(/does not match a key in the current Paseo Cloud account/),
    ).not.toBeNull();

    fireEvent.click(screen.getByTestId("paseo-cloud-route-repair-routing-claude-code"));
    expect(screen.queryByText("Routing section content")).not.toBeNull();

    fireEvent.click(screen.getByTestId("paseo-cloud-section-overview"));
    fireEvent.click(screen.getByTestId("paseo-cloud-route-repair-keys-claude-code"));
    expect(screen.queryByText("API keys section content")).not.toBeNull();
  });
});
