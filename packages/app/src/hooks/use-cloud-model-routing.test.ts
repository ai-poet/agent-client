import { describe, expect, it } from "vitest";
import type { AgentProviderDefinition } from "@server/server/agent/provider-manifest";
import type {
  Sub2APIGroup,
  Sub2APIGroupStatusItem,
  Sub2APIKey,
  Sub2APIModelCatalog,
} from "@/lib/sub2api-client";
import type { ProviderStore } from "@/screens/settings/sub2api-provider-types";
import {
  buildCloudModelRoutingGroups,
  buildGlobalCloudRouteGroups,
  resolveActiveGlobalCloudProviders,
} from "./cloud-model-routing-utils";

const providerDefinitions: AgentProviderDefinition[] = [
  {
    id: "claude",
    label: "Claude",
    description: "Claude provider",
    defaultModeId: "default",
    modes: [],
  },
  {
    id: "codex",
    label: "Codex",
    description: "Codex provider",
    defaultModeId: "auto",
    modes: [],
  },
];

const price = {
  input_per_mtok_usd: 1,
  output_per_mtok_usd: 2,
  cache_write_per_mtok_usd: null,
  cache_read_per_mtok_usd: null,
  per_request_usd: null,
  per_image_usd: null,
  source: "official",
  has_reference: true,
};

const catalog: Sub2APIModelCatalog = {
  summary: {
    total_models: 1,
    token_models: 1,
    non_token_models: 0,
    best_savings_model: "claude-sonnet",
    max_savings_percent: 0.1,
  },
  items: [
    {
      model: "claude-sonnet",
      display_name: "Claude Sonnet",
      platform: "anthropic",
      billing_mode: "token",
      best_group: {
        id: 10,
        name: "Claude Fast",
        rate_multiplier: 0.8,
        rate_source: "group",
      },
      available_group_count: 1,
      official_pricing: price,
      effective_pricing_usd: price,
      comparison: {
        savings_percent: 0.2,
        is_cheaper_than_official: true,
        delta_input_per_mtok_usd: null,
        delta_output_per_mtok_usd: null,
        delta_per_request_usd: null,
        delta_per_image_usd: null,
      },
      pricing_details: {
        supports_prompt_caching: false,
        has_long_context_multiplier: false,
        long_context_input_threshold: 0,
        intervals: [],
      },
      other_groups: [],
    },
  ],
};

const statuses: Sub2APIGroupStatusItem[] = [
  {
    group_id: 10,
    group_name: "Claude Fast",
    latest_status: "up",
    stable_status: "up",
    latency_ms: null,
    availability_24h: 99.5,
    availability_7d: 99,
    observed_at: null,
  },
];

const claudeGroup: Sub2APIGroup = {
  id: 10,
  name: "Claude Fast",
  description: "",
  platform: "anthropic",
  rate_multiplier: 0.8,
  status: "active",
  subscription_type: "pay_as_you_go",
  allow_messages_dispatch: true,
};

describe("buildCloudModelRoutingGroups", () => {
  it("builds selector cloud groups from the catalog and marks the current global key group", () => {
    const groups = buildCloudModelRoutingGroups({
      catalog,
      statuses,
      providerDefinitions,
      activeGroupIdsByProvider: {
        claude: 10,
      },
    });

    expect(groups).toEqual([
      expect.objectContaining({
        provider: "claude",
        groupId: 10,
        groupLabel: "Claude Fast",
        platform: "anthropic",
        isActiveForGlobalKey: true,
        description: expect.stringContaining("Current global key"),
        models: [
          expect.objectContaining({
            id: "claude-sonnet",
            label: "Claude Sonnet",
          }),
        ],
      }),
    ]);
  });
});

describe("resolveActiveGlobalCloudProviders", () => {
  it("resolves scoped active Claude and Codex providers from the desktop store", () => {
    const store: ProviderStore = {
      activeProviderId: null,
      activeClaudeProviderId: "claude-cloud",
      activeCodexProviderId: "codex-cloud",
      providers: [
        {
          id: "claude-cloud",
          name: "Cloud Claude",
          type: "default",
          endpoint: "https://cloud.example.com/v1",
          apiKey: " sk-claude ",
          isDefault: true,
          target: "claude",
        },
        {
          id: "codex-cloud",
          name: "Cloud Codex",
          type: "default",
          endpoint: "https://cloud.example.com",
          apiKey: "sk-codex",
          isDefault: true,
          target: "codex",
        },
      ],
    };

    expect(resolveActiveGlobalCloudProviders(store)).toEqual([
      expect.objectContaining({
        provider: "claude",
        apiKey: "sk-claude",
        endpoint: "https://cloud.example.com",
      }),
      expect.objectContaining({
        provider: "codex",
        apiKey: "sk-codex",
        endpoint: "https://cloud.example.com",
      }),
    ]);
  });
});

describe("buildGlobalCloudRouteGroups", () => {
  it("shows the group that owns the active global Cloud key", () => {
    const keys: Sub2APIKey[] = [
      {
        id: 7,
        user_id: 1,
        key: "sk-global",
        name: "Desktop Claude",
        group_id: claudeGroup.id,
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
        group: claudeGroup,
      },
    ];

    const groups = buildGlobalCloudRouteGroups({
      activeProviders: [
        {
          provider: "claude",
          apiKey: "sk-global",
          endpoint: "https://cloud.example.com",
        },
      ],
      cloudEndpoint: "https://cloud.example.com/v1",
      keys,
      groups: [claudeGroup],
      providerDefinitions,
      allProviderModels: new Map([
        [
          "claude",
          [
            {
              provider: "claude",
              id: "claude-sonnet-5",
              label: "Sonnet 5",
              description: "Managed catalog model",
            },
          ],
        ],
      ]),
    });

    expect(groups).toEqual([
      expect.objectContaining({
        provider: "claude",
        groupId: 10,
        groupLabel: "Claude Fast",
        isActiveForGlobalKey: true,
        models: [
          {
            id: "claude-sonnet-5",
            label: "Sonnet 5",
            description: "Managed catalog model",
          },
        ],
      }),
    ]);
  });

  it("omits a global Cloud group until its provider snapshot has models", () => {
    const groups = buildGlobalCloudRouteGroups({
      activeProviders: [
        {
          provider: "claude",
          apiKey: "sk-global",
          endpoint: "https://cloud.example.com",
        },
      ],
      cloudEndpoint: "https://cloud.example.com",
      keys: [
        {
          id: 7,
          user_id: 1,
          key: "sk-global",
          name: "Desktop Claude",
          group_id: claudeGroup.id,
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
          group: claudeGroup,
        },
      ],
      groups: [claudeGroup],
      providerDefinitions,
      allProviderModels: new Map(),
    });

    expect(groups).toEqual([]);
  });
});
