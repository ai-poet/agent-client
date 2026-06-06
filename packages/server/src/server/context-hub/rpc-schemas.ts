import { z } from "zod";

export const ProjectMemoryKindSchema = z.enum([
  "project_context",
  "architecture_decision",
  "known_issue",
  "setup_instruction",
  "coding_convention",
  "user_preference",
  "api_contract",
  "workflow_command",
  "task_followup",
  "note",
]);

export const MemoryImportanceSchema = z.enum(["high", "medium", "low"]);

export const ProjectMemoryItemSchema = z.object({
  id: z.string(),
  workspaceId: z.string(),
  kind: ProjectMemoryKindSchema,
  title: z.string(),
  summary: z.string(),
  detail: z.string().nullable(),
  rawText: z.string().nullable(),
  cleanText: z.string(),
  tags: z.array(z.string()),
  importance: MemoryImportanceSchema,
  source: z.string(),
  fingerprint: z.string(),
  threadId: z.string().nullable(),
  messageId: z.string().nullable(),
  metadata: z.record(z.unknown()).optional(),
  createdAt: z.string(),
  updatedAt: z.string(),
  deletedAt: z.string().nullable(),
});

export const ProjectMemoryCreateInputSchema = z.object({
  workspaceId: z.string().min(1),
  kind: ProjectMemoryKindSchema.optional(),
  title: z.string().optional(),
  summary: z.string().optional(),
  detail: z.string().optional(),
  rawText: z.string().optional(),
  tags: z.array(z.string()).optional(),
  importance: MemoryImportanceSchema.optional(),
  source: z.string().optional(),
  threadId: z.string().optional(),
  messageId: z.string().optional(),
  metadata: z.record(z.unknown()).optional(),
});

export const ProjectMemoryUpdateInputSchema = ProjectMemoryCreateInputSchema.omit({
  workspaceId: true,
})
  .partial()
  .extend({
    deletedAt: z.string().nullable().optional(),
  });

export const ProjectMemorySettingsSchema = z.object({
  autoInjectEnabled: z.boolean().default(false),
  dedupeEnabled: z.boolean().default(true),
  desensitizeEnabled: z.boolean().default(true),
  workspaceOverrides: z
    .record(
      z.object({
        autoInjectEnabled: z.boolean().optional(),
      }),
    )
    .default({}),
});

const ContextHubMcpStdioServerConfigSchema = z.object({
  type: z.literal("stdio"),
  command: z.string().min(1),
  args: z.array(z.string()).optional(),
  env: z.record(z.string()).optional(),
});

const ContextHubMcpHttpServerConfigSchema = z.object({
  type: z.literal("http"),
  url: z.string().min(1),
  headers: z.record(z.string()).optional(),
});

const ContextHubMcpSseServerConfigSchema = z.object({
  type: z.literal("sse"),
  url: z.string().min(1),
  headers: z.record(z.string()).optional(),
});

export const ContextHubMcpServerConfigSchema = z.discriminatedUnion("type", [
  ContextHubMcpStdioServerConfigSchema,
  ContextHubMcpHttpServerConfigSchema,
  ContextHubMcpSseServerConfigSchema,
]);

export const PromptTemplateSchema = z.object({
  id: z.string(),
  scope: z.enum(["global", "workspace"]),
  workspaceId: z.string().nullable(),
  name: z.string(),
  description: z.string().nullable(),
  content: z.string(),
  tags: z.array(z.string()),
  variables: z.array(z.string()),
  usageCount: z.number().int().nonnegative(),
  lastUsedAt: z.string().nullable(),
  source: z.enum(["managed", "codex"]),
  readOnly: z.boolean(),
  path: z.string().nullable(),
  createdAt: z.string(),
  updatedAt: z.string(),
  deletedAt: z.string().nullable(),
});

export const PromptTemplateCreateInputSchema = z.object({
  scope: z.enum(["global", "workspace"]).default("global"),
  workspaceId: z.string().optional(),
  name: z.string().min(1),
  description: z.string().optional(),
  content: z.string().min(1),
  tags: z.array(z.string()).optional(),
});

export const PromptTemplateUpdateInputSchema = PromptTemplateCreateInputSchema.partial().extend({
  deletedAt: z.string().nullable().optional(),
});

export const ManagedSkillEntrySchema = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string().nullable(),
  source: z.string(),
  scope: z.enum(["global", "workspace"]),
  workspaceId: z.string().nullable(),
  path: z.string(),
  readOnly: z.boolean(),
  content: z.string().nullable().optional(),
  createdAt: z.string().nullable(),
  updatedAt: z.string().nullable(),
});

export const SkillWritableTargetSchema = z.enum([
  "managed",
  "global_codex",
  "global_claude",
  "global_agents",
  "project_codex",
  "project_claude",
  "project_agents",
]);

export const SkillScopeSchema = z.enum(["global", "workspace"]);

export const SkillMarketplaceTrustLevelSchema = z.enum(["verified", "community", "sandbox"]);

export const MarketplaceSkillPermissionsSchema = z.object({
  network: z.boolean().optional(),
  filesystem: z.boolean().optional(),
  subprocess: z.boolean().optional(),
  envVars: z.array(z.string()).default([]),
});

export const MarketplaceSkillEntrySchema = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string(),
  version: z.string().nullable(),
  trustLevel: SkillMarketplaceTrustLevelSchema,
  vettingStatus: z.string().nullable(),
  capabilities: z.array(z.string()),
  permissions: MarketplaceSkillPermissionsSchema,
  platformCompatibility: z.array(z.string()),
  downloadCount: z.number().int().nonnegative(),
  downloads7d: z.number().int().nonnegative(),
  daysSinceUpdate: z.number().int().nonnegative().nullable(),
  installed: z.boolean(),
});

export const McpServerProfileSchema = z.object({
  id: z.string(),
  name: z.string(),
  enabled: z.boolean(),
  providerIds: z.array(z.string()).optional(),
  workspaceIds: z.array(z.string()).optional(),
  server: ContextHubMcpServerConfigSchema,
  createdAt: z.string(),
  updatedAt: z.string(),
});

export const McpServerProfileUpsertInputSchema = z.object({
  id: z.string().optional(),
  name: z.string().min(1),
  enabled: z.boolean().optional(),
  providerIds: z.array(z.string()).optional(),
  workspaceIds: z.array(z.string()).optional(),
  server: ContextHubMcpServerConfigSchema,
});

const RequestIdSchema = z.string();

export const MemoryListRequestSchema = z.object({
  type: z.literal("memory/list"),
  requestId: RequestIdSchema,
  workspaceId: z.string().min(1),
  query: z.string().optional(),
  kind: ProjectMemoryKindSchema.optional(),
  importance: MemoryImportanceSchema.optional(),
  tag: z.string().optional(),
  includeDeleted: z.boolean().optional(),
  page: z.number().int().nonnegative().optional(),
  pageSize: z.number().int().positive().max(200).optional(),
});

export const MemoryGetRequestSchema = z.object({
  type: z.literal("memory/get"),
  requestId: RequestIdSchema,
  workspaceId: z.string().min(1),
  memoryId: z.string().min(1),
});

export const MemoryCreateRequestSchema = z.object({
  type: z.literal("memory/create"),
  requestId: RequestIdSchema,
  input: ProjectMemoryCreateInputSchema,
});

export const MemoryUpdateRequestSchema = z.object({
  type: z.literal("memory/update"),
  requestId: RequestIdSchema,
  workspaceId: z.string().min(1),
  memoryId: z.string().min(1),
  patch: ProjectMemoryUpdateInputSchema,
});

export const MemoryDeleteRequestSchema = z.object({
  type: z.literal("memory/delete"),
  requestId: RequestIdSchema,
  workspaceId: z.string().min(1),
  memoryId: z.string().min(1),
});

export const SkillsListRequestSchema = z.object({
  type: z.literal("skills/list"),
  requestId: RequestIdSchema,
  workspaceId: z.string().optional(),
  cwd: z.string().optional(),
  includeContent: z.boolean().optional(),
});

export const SkillsImportRequestSchema = z.object({
  type: z.literal("skills/import"),
  requestId: RequestIdSchema,
  name: z.string().min(1),
  content: z.string().min(1),
  description: z.string().optional(),
});

export const SkillsExportRequestSchema = z.object({
  type: z.literal("skills/export"),
  requestId: RequestIdSchema,
  skillId: z.string().min(1),
  workspaceId: z.string().optional(),
  cwd: z.string().optional(),
});

export const SkillsSaveRequestSchema = z.object({
  type: z.literal("skills/save"),
  requestId: RequestIdSchema,
  target: SkillWritableTargetSchema,
  scope: SkillScopeSchema,
  skillId: z.string().optional(),
  name: z.string().min(1),
  content: z.string().min(1),
  workspaceId: z.string().optional(),
  cwd: z.string().optional(),
  overwrite: z.boolean().optional(),
});

export const SkillsImportPackageRequestSchema = z.object({
  type: z.literal("skills/import-package"),
  requestId: RequestIdSchema,
  target: SkillWritableTargetSchema,
  scope: SkillScopeSchema,
  name: z.string().min(1),
  packageBase64: z.string().min(1),
  workspaceId: z.string().optional(),
  cwd: z.string().optional(),
  overwrite: z.boolean().optional(),
});

export const SkillsDeleteRequestSchema = z.object({
  type: z.literal("skills/delete"),
  requestId: RequestIdSchema,
  skillId: z.string().min(1),
  workspaceId: z.string().optional(),
  cwd: z.string().optional(),
});

export const SkillsMarketplaceListRequestSchema = z.object({
  type: z.literal("skills/marketplace/list"),
  requestId: RequestIdSchema,
  query: z.string().optional(),
  capability: z.string().optional(),
  limit: z.number().int().positive().max(50).optional(),
  minTrust: SkillMarketplaceTrustLevelSchema.optional(),
  refresh: z.boolean().optional(),
  workspaceId: z.string().optional(),
  cwd: z.string().optional(),
});

export const SkillsMarketplaceInstallRequestSchema = z.object({
  type: z.literal("skills/marketplace/install"),
  requestId: RequestIdSchema,
  workspaceId: z.string().min(1),
  cwd: z.string().optional(),
  skillId: z.string().min(1),
  name: z.string().min(1),
  version: z.string().optional(),
  overwrite: z.boolean().optional(),
});

export const PromptsListRequestSchema = z.object({
  type: z.literal("prompts/list"),
  requestId: RequestIdSchema,
  workspaceId: z.string().optional(),
  includeDeleted: z.boolean().optional(),
});

export const PromptsCreateRequestSchema = z.object({
  type: z.literal("prompts/create"),
  requestId: RequestIdSchema,
  input: PromptTemplateCreateInputSchema,
});

export const PromptsUpdateRequestSchema = z.object({
  type: z.literal("prompts/update"),
  requestId: RequestIdSchema,
  promptId: z.string().min(1),
  patch: PromptTemplateUpdateInputSchema,
});

export const PromptsDeleteRequestSchema = z.object({
  type: z.literal("prompts/delete"),
  requestId: RequestIdSchema,
  promptId: z.string().min(1),
});

export const PromptsRenderRequestSchema = z.object({
  type: z.literal("prompts/render"),
  requestId: RequestIdSchema,
  promptId: z.string().min(1),
  variables: z.record(z.string()).optional(),
  argumentsText: z.string().optional(),
  recordUsage: z.boolean().optional(),
});

export const McpListRequestSchema = z.object({
  type: z.literal("mcp/list"),
  requestId: RequestIdSchema,
});

export const McpUpsertRequestSchema = z.object({
  type: z.literal("mcp/upsert"),
  requestId: RequestIdSchema,
  profile: McpServerProfileUpsertInputSchema,
});

export const McpDeleteRequestSchema = z.object({
  type: z.literal("mcp/delete"),
  requestId: RequestIdSchema,
  profileId: z.string().min(1),
});

export const McpTestRequestSchema = z.object({
  type: z.literal("mcp/test"),
  requestId: RequestIdSchema,
  profile: McpServerProfileUpsertInputSchema,
});

export const MemoryListResponseSchema = z.object({
  type: z.literal("memory/list/response"),
  payload: z.object({
    requestId: z.string(),
    items: z.array(ProjectMemoryItemSchema),
    total: z.number().int().nonnegative(),
    error: z.string().nullable(),
  }),
});

export const MemoryGetResponseSchema = z.object({
  type: z.literal("memory/get/response"),
  payload: z.object({
    requestId: z.string(),
    item: ProjectMemoryItemSchema.nullable(),
    error: z.string().nullable(),
  }),
});

export const MemoryCreateResponseSchema = z.object({
  type: z.literal("memory/create/response"),
  payload: z.object({
    requestId: z.string(),
    item: ProjectMemoryItemSchema.nullable(),
    error: z.string().nullable(),
  }),
});

export const MemoryUpdateResponseSchema = z.object({
  type: z.literal("memory/update/response"),
  payload: z.object({
    requestId: z.string(),
    item: ProjectMemoryItemSchema.nullable(),
    error: z.string().nullable(),
  }),
});

export const MemoryDeleteResponseSchema = z.object({
  type: z.literal("memory/delete/response"),
  payload: z.object({
    requestId: z.string(),
    item: ProjectMemoryItemSchema.nullable(),
    error: z.string().nullable(),
  }),
});

export const SkillsListResponseSchema = z.object({
  type: z.literal("skills/list/response"),
  payload: z.object({
    requestId: z.string(),
    skills: z.array(ManagedSkillEntrySchema),
    error: z.string().nullable(),
  }),
});

export const SkillsImportResponseSchema = z.object({
  type: z.literal("skills/import/response"),
  payload: z.object({
    requestId: z.string(),
    skill: ManagedSkillEntrySchema.nullable(),
    error: z.string().nullable(),
  }),
});

export const SkillsExportResponseSchema = z.object({
  type: z.literal("skills/export/response"),
  payload: z.object({
    requestId: z.string(),
    skill: ManagedSkillEntrySchema.nullable(),
    content: z.string().nullable(),
    error: z.string().nullable(),
  }),
});

export const SkillsSaveResponseSchema = z.object({
  type: z.literal("skills/save/response"),
  payload: z.object({
    requestId: z.string(),
    skill: ManagedSkillEntrySchema.nullable(),
    conflict: z.boolean(),
    error: z.string().nullable(),
  }),
});

export const SkillsImportPackageResponseSchema = z.object({
  type: z.literal("skills/import-package/response"),
  payload: z.object({
    requestId: z.string(),
    skill: ManagedSkillEntrySchema.nullable(),
    conflict: z.boolean(),
    error: z.string().nullable(),
  }),
});

export const SkillsDeleteResponseSchema = z.object({
  type: z.literal("skills/delete/response"),
  payload: z.object({
    requestId: z.string(),
    skillId: z.string(),
    error: z.string().nullable(),
  }),
});

export const SkillsMarketplaceListResponseSchema = z.object({
  type: z.literal("skills/marketplace/list/response"),
  payload: z.object({
    requestId: z.string(),
    skills: z.array(MarketplaceSkillEntrySchema),
    error: z.string().nullable(),
  }),
});

export const SkillsMarketplaceInstallResponseSchema = z.object({
  type: z.literal("skills/marketplace/install/response"),
  payload: z.object({
    requestId: z.string(),
    skill: ManagedSkillEntrySchema.nullable(),
    installed: z.boolean(),
    conflict: z.boolean(),
    error: z.string().nullable(),
  }),
});

export const PromptsListResponseSchema = z.object({
  type: z.literal("prompts/list/response"),
  payload: z.object({
    requestId: z.string(),
    prompts: z.array(PromptTemplateSchema),
    error: z.string().nullable(),
  }),
});

export const PromptsCreateResponseSchema = z.object({
  type: z.literal("prompts/create/response"),
  payload: z.object({
    requestId: z.string(),
    prompt: PromptTemplateSchema.nullable(),
    error: z.string().nullable(),
  }),
});

export const PromptsUpdateResponseSchema = z.object({
  type: z.literal("prompts/update/response"),
  payload: z.object({
    requestId: z.string(),
    prompt: PromptTemplateSchema.nullable(),
    error: z.string().nullable(),
  }),
});

export const PromptsDeleteResponseSchema = z.object({
  type: z.literal("prompts/delete/response"),
  payload: z.object({
    requestId: z.string(),
    promptId: z.string(),
    error: z.string().nullable(),
  }),
});

export const PromptsRenderResponseSchema = z.object({
  type: z.literal("prompts/render/response"),
  payload: z.object({
    requestId: z.string(),
    text: z.string().nullable(),
    prompt: PromptTemplateSchema.nullable(),
    error: z.string().nullable(),
  }),
});

export const McpListResponseSchema = z.object({
  type: z.literal("mcp/list/response"),
  payload: z.object({
    requestId: z.string(),
    profiles: z.array(McpServerProfileSchema),
    error: z.string().nullable(),
  }),
});

export const McpUpsertResponseSchema = z.object({
  type: z.literal("mcp/upsert/response"),
  payload: z.object({
    requestId: z.string(),
    profile: McpServerProfileSchema.nullable(),
    error: z.string().nullable(),
  }),
});

export const McpDeleteResponseSchema = z.object({
  type: z.literal("mcp/delete/response"),
  payload: z.object({
    requestId: z.string(),
    profileId: z.string(),
    error: z.string().nullable(),
  }),
});

export const McpTestResponseSchema = z.object({
  type: z.literal("mcp/test/response"),
  payload: z.object({
    requestId: z.string(),
    ok: z.boolean(),
    message: z.string(),
    error: z.string().nullable(),
  }),
});

export type ProjectMemoryKind = z.infer<typeof ProjectMemoryKindSchema>;
export type ProjectMemoryItem = z.infer<typeof ProjectMemoryItemSchema>;
export type ProjectMemoryCreateInput = z.infer<typeof ProjectMemoryCreateInputSchema>;
export type ProjectMemoryUpdateInput = z.infer<typeof ProjectMemoryUpdateInputSchema>;
export type ProjectMemorySettings = z.infer<typeof ProjectMemorySettingsSchema>;
export type PromptTemplate = z.infer<typeof PromptTemplateSchema>;
export type PromptTemplateCreateInput = z.infer<typeof PromptTemplateCreateInputSchema>;
export type PromptTemplateUpdateInput = z.infer<typeof PromptTemplateUpdateInputSchema>;
export type ManagedSkillEntry = z.infer<typeof ManagedSkillEntrySchema>;
export type SkillWritableTarget = z.infer<typeof SkillWritableTargetSchema>;
export type SkillScope = z.infer<typeof SkillScopeSchema>;
export type SkillMarketplaceTrustLevel = z.infer<typeof SkillMarketplaceTrustLevelSchema>;
export type MarketplaceSkillEntry = z.infer<typeof MarketplaceSkillEntrySchema>;
export type McpServerProfile = z.infer<typeof McpServerProfileSchema>;
export type McpServerProfileUpsertInput = z.infer<typeof McpServerProfileUpsertInputSchema>;
export type ContextHubMcpServerConfig = z.infer<typeof ContextHubMcpServerConfigSchema>;
