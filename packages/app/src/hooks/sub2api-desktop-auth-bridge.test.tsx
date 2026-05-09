/**
 * @vitest-environment jsdom
 */
import React from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Sub2apiDesktopAuthBridge } from "./sub2api-desktop-auth-bridge";

type AuthCallbackListener = (payload: { url: string }) => void;

const { mocks } = vi.hoisted(() => {
  let listener: AuthCallbackListener | null = null;
  return {
    mocks: {
      login: vi.fn(),
      getAccessToken: vi.fn(),
      invokeDesktopCommand: vi.fn(),
      setListener(next: AuthCallbackListener | null) {
        listener = next;
      },
      getListener() {
        return listener;
      },
    },
  };
});

vi.mock("@/constants/platform", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/constants/platform")>();
  return {
    ...actual,
    getIsElectron: () => true,
  };
});

vi.mock("@/hooks/use-sub2api-auth", () => ({
  useSub2APIAuth: () => ({
    auth: null,
    isLoggedIn: false,
    isLoading: false,
    login: mocks.login,
    logout: vi.fn(),
    getAccessToken: mocks.getAccessToken,
  }),
}));

vi.mock("@/desktop/electron/invoke", () => ({
  invokeDesktopCommand: (...args: unknown[]) => mocks.invokeDesktopCommand(...args),
}));

vi.mock("@/desktop/host", () => ({
  getDesktopHost: () => ({
    events: {
      on: async (_event: string, cb: AuthCallbackListener) => {
        mocks.setListener(cb);
        return () => mocks.setListener(null);
      },
    },
    getPendingAuthCallback: async () => null,
  }),
}));

describe("Sub2apiDesktopAuthBridge", () => {
  beforeEach(() => {
    mocks.login.mockReset();
    mocks.getAccessToken.mockReset();
    mocks.getAccessToken.mockResolvedValue("at");
    mocks.invokeDesktopCommand.mockReset();
    mocks.invokeDesktopCommand.mockImplementation(async (command: string) => {
      if (command === "get_providers") {
        return {
          providers: [],
          activeProviderId: null,
          activeClaudeProviderId: null,
          activeCodexProviderId: null,
        };
      }
      return null;
    });
    mocks.setListener(null);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("persists auth and auto-configures missing scoped routes from the Electron auth-callback", async () => {
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });

    render(
      <QueryClientProvider client={queryClient}>
        <Sub2apiDesktopAuthBridge />
      </QueryClientProvider>,
    );

    await waitFor(() => {
      expect(mocks.getListener()).not.toBeNull();
    });

    const callbackUrl =
      "https://desktop.example/auth/paseo#access_token=at&refresh_token=rt&api_key=sk-live&claude_api_key=sk-claude&codex_api_key=sk-codex&endpoint=https%3A%2F%2Fapi.example.com&expires_in=3600";

    mocks.getListener()?.({ url: callbackUrl });

    await waitFor(() => {
      expect(mocks.login).toHaveBeenCalledWith({
        accessToken: "at",
        refreshToken: "rt",
        expiresIn: 3600,
        endpoint: "https://api.example.com",
      });
    });

    await waitFor(() => {
      expect(mocks.invokeDesktopCommand).toHaveBeenCalledWith("setup_default_provider", {
        endpoint: "https://api.example.com",
        apiKey: "sk-claude",
        scope: "claude",
        name: "Paseo Cloud",
      });
      expect(mocks.invokeDesktopCommand).toHaveBeenCalledWith("setup_default_provider", {
        endpoint: "https://api.example.com",
        apiKey: "sk-codex",
        scope: "codex",
        name: "Paseo Cloud",
      });
    });
  });

  it("creates cloud keys and configures local routes when the callback has no scoped keys", async () => {
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url === "https://api.example.com/api/v1/groups/available") {
        return new Response(
          JSON.stringify({
            code: 0,
            message: "ok",
            data: [
              {
                id: 11,
                name: "Claude Group",
                description: "",
                platform: "anthropic",
                rate_multiplier: 1,
                status: "active",
                subscription_type: "",
                allow_messages_dispatch: true,
              },
              {
                id: 22,
                name: "Codex Group",
                description: "",
                platform: "openai",
                rate_multiplier: 1,
                status: "active",
                subscription_type: "",
                allow_messages_dispatch: true,
              },
            ],
          }),
        );
      }
      if (url === "https://api.example.com/api/v1/keys?page=1&page_size=200") {
        return new Response(
          JSON.stringify({
            code: 0,
            message: "ok",
            data: { items: [], total: 0, page: 1, page_size: 200, pages: 0 },
          }),
        );
      }
      if (url === "https://api.example.com/api/v1/keys" && init?.method === "POST") {
        const body = JSON.parse(String(init.body)) as { group_id: number; name: string };
        return new Response(
          JSON.stringify({
            code: 0,
            message: "ok",
            data: {
              id: body.group_id === 11 ? 101 : 202,
              user_id: 1,
              key: body.group_id === 11 ? "sk-created-claude" : "sk-created-codex",
              name: body.name,
              group_id: body.group_id,
              status: "active",
              quota: 0,
              quota_used: 0,
              rate_limit_5h: 0,
              rate_limit_1d: 0,
              rate_limit_7d: 0,
              usage_5h: 0,
              usage_1d: 0,
              usage_7d: 0,
              created_at: "",
              updated_at: "",
            },
          }),
        );
      }
      throw new Error(`Unexpected request: ${url}`);
    });
    vi.stubGlobal("fetch", fetchMock);

    render(
      <QueryClientProvider client={queryClient}>
        <Sub2apiDesktopAuthBridge />
      </QueryClientProvider>,
    );

    await waitFor(() => {
      expect(mocks.getListener()).not.toBeNull();
    });

    const callbackUrl =
      "https://desktop.example/auth/paseo#access_token=at&refresh_token=rt&endpoint=https%3A%2F%2Fapi.example.com&expires_in=3600";

    mocks.getListener()?.({ url: callbackUrl });

    await waitFor(() => {
      expect(mocks.invokeDesktopCommand).toHaveBeenCalledWith("setup_default_provider", {
        endpoint: "https://api.example.com",
        apiKey: "sk-created-claude",
        scope: "claude",
        name: "Claude Group",
      });
      expect(mocks.invokeDesktopCommand).toHaveBeenCalledWith("setup_default_provider", {
        endpoint: "https://api.example.com",
        apiKey: "sk-created-codex",
        scope: "codex",
        name: "Codex Group",
      });
    });

    expect(fetchMock).toHaveBeenCalledWith(
      "https://api.example.com/api/v1/keys",
      expect.objectContaining({
        method: "POST",
        headers: expect.objectContaining({ Authorization: "Bearer at" }),
        body: JSON.stringify({ name: "Claude Group Key", group_id: 11 }),
      }),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "https://api.example.com/api/v1/keys",
      expect.objectContaining({
        method: "POST",
        headers: expect.objectContaining({ Authorization: "Bearer at" }),
        body: JSON.stringify({ name: "Codex Group Key", group_id: 22 }),
      }),
    );
  });

  it("reuses an existing cloud key when a local active route is not using the signed-in cloud account", async () => {
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    mocks.invokeDesktopCommand.mockImplementation(async (command: string) => {
      if (command === "get_providers") {
        return {
          providers: [
            {
              id: "local-claude",
              name: "Local Claude",
              type: "custom",
              endpoint: "https://byok.example.com",
              apiKey: "sk-local",
              isDefault: false,
              target: "claude",
            },
          ],
          activeProviderId: null,
          activeClaudeProviderId: "local-claude",
          activeCodexProviderId: null,
        };
      }
      return null;
    });
    const fetchMock = vi.fn(async (url: string) => {
      if (url === "https://api.example.com/api/v1/groups/available") {
        return new Response(
          JSON.stringify({
            code: 0,
            message: "ok",
            data: [
              {
                id: 11,
                name: "Claude Group",
                description: "",
                platform: "anthropic",
                rate_multiplier: 1,
                status: "active",
                subscription_type: "",
                allow_messages_dispatch: true,
              },
            ],
          }),
        );
      }
      if (url === "https://api.example.com/api/v1/keys?page=1&page_size=200") {
        return new Response(
          JSON.stringify({
            code: 0,
            message: "ok",
            data: {
              items: [
                {
                  id: 101,
                  user_id: 1,
                  key: "sk-existing-cloud",
                  name: "Existing",
                  group_id: 11,
                  status: "active",
                  quota: 0,
                  quota_used: 0,
                  rate_limit_5h: 0,
                  rate_limit_1d: 0,
                  rate_limit_7d: 0,
                  usage_5h: 0,
                  usage_1d: 0,
                  usage_7d: 0,
                  created_at: "",
                  updated_at: "",
                },
              ],
              total: 1,
              page: 1,
              page_size: 200,
              pages: 1,
            },
          }),
        );
      }
      throw new Error(`Unexpected request: ${url}`);
    });
    vi.stubGlobal("fetch", fetchMock);

    render(
      <QueryClientProvider client={queryClient}>
        <Sub2apiDesktopAuthBridge />
      </QueryClientProvider>,
    );

    await waitFor(() => {
      expect(mocks.getListener()).not.toBeNull();
    });

    const callbackUrl =
      "https://desktop.example/auth/paseo#access_token=at&refresh_token=rt&api_key=sk-live&endpoint=https%3A%2F%2Fapi.example.com&expires_in=3600";

    mocks.getListener()?.({ url: callbackUrl });

    await waitFor(() => {
      expect(mocks.invokeDesktopCommand).toHaveBeenCalledWith("setup_default_provider", {
        endpoint: "https://api.example.com",
        apiKey: "sk-existing-cloud",
        scope: "claude",
        name: "Claude Group",
      });
    });

    expect(fetchMock).not.toHaveBeenCalledWith(
      "https://api.example.com/api/v1/keys",
      expect.objectContaining({ method: "POST" }),
    );
  });
});
