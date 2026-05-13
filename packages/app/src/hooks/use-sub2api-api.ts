import { useMemo } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useSub2APIAuth } from "@/hooks/use-sub2api-auth";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { normalizeSub2APILocale } from "@/i18n/sub2api";
import {
  createSub2APIClient,
  normalizeSub2APIEndpoint,
  type Sub2APICreateKeyRequest,
  type Sub2APIModelCatalog,
  type Sub2APIPaginatedData,
  type Sub2APIGroupStatusHistoryPeriod,
  type Sub2APIUsagePeriod,
  type Sub2APIUsageLogsQuery,
  type Sub2APIUsageStatsQuery,
  type Sub2APIUsageStats,
  type Sub2APIUsageLog,
  type Sub2APIUser,
  type Sub2APIKey,
  type Sub2APIGroup,
  type Sub2APIClient,
  type Sub2APIUpdateKeyRequest,
  type Sub2APIGroupStatusItem,
  type Sub2APIGroupStatusHistoryBucket,
  type Sub2APIGroupStatusRecord,
  type Sub2APIGroupStatusEvent,
  type Sub2APIReferralInfo,
  type Sub2APIUserReferral,
  type Sub2APIRedeemResult,
} from "@/lib/sub2api-client";

export interface Sub2APIPaymentConfig {
  balanceCreditCnyPerUsd: number | null;
  usdExchangeRate: number | null;
}

function normalizeEndpointKey(endpoint: string | null | undefined): string {
  const trimmed = endpoint?.trim();
  return trimmed && trimmed.length > 0 ? trimmed : "none";
}

function normalizeSessionKey(sessionKey: string | null | undefined): string {
  const trimmed = sessionKey?.trim();
  return trimmed && trimmed.length > 0 ? trimmed : "anonymous";
}

function normalizeUsageQuery(
  input: Sub2APIUsagePeriod | Sub2APIUsageStatsQuery,
): Required<Sub2APIUsageStatsQuery> {
  if (typeof input === "string") {
    return {
      period: input,
      apiKeyId: null,
      timezone: "",
      startDate: "",
      endDate: "",
    };
  }
  return {
    period: input.period ?? "today",
    apiKeyId: typeof input.apiKeyId === "number" ? input.apiKeyId : null,
    timezone: input.timezone?.trim() ?? "",
    startDate: input.startDate?.trim() ?? "",
    endDate: input.endDate?.trim() ?? "",
  };
}

function normalizeUsageLogsQuery(input: Sub2APIUsageLogsQuery): Required<Sub2APIUsageLogsQuery> {
  return {
    page: typeof input.page === "number" ? input.page : 1,
    pageSize: typeof input.pageSize === "number" ? input.pageSize : 20,
    apiKeyId: typeof input.apiKeyId === "number" ? input.apiKeyId : null,
    startDate: input.startDate?.trim() ?? "",
    endDate: input.endDate?.trim() ?? "",
  };
}

const CLOUD_SERVICE_QUERY_ROOT = "managedCloud";

export const cloudServiceQueryKeys = {
  root: [CLOUD_SERVICE_QUERY_ROOT] as const,
  me: (endpoint: string | null | undefined, sessionKey: string | null | undefined) =>
    [
      CLOUD_SERVICE_QUERY_ROOT,
      normalizeSessionKey(sessionKey),
      normalizeEndpointKey(endpoint),
      "me",
    ] as const,
  keys: (endpoint: string | null | undefined, sessionKey: string | null | undefined) =>
    [
      CLOUD_SERVICE_QUERY_ROOT,
      normalizeSessionKey(sessionKey),
      normalizeEndpointKey(endpoint),
      "keys",
    ] as const,
  groups: (endpoint: string | null | undefined, sessionKey: string | null | undefined) =>
    [
      CLOUD_SERVICE_QUERY_ROOT,
      normalizeSessionKey(sessionKey),
      normalizeEndpointKey(endpoint),
      "groups",
    ] as const,
  usage: (
    endpoint: string | null | undefined,
    sessionKey: string | null | undefined,
    query: Sub2APIUsagePeriod | Sub2APIUsageStatsQuery,
  ) => {
    const normalized = normalizeUsageQuery(query);
    return [
      CLOUD_SERVICE_QUERY_ROOT,
      normalizeSessionKey(sessionKey),
      normalizeEndpointKey(endpoint),
      "usage",
      normalized.period,
      normalized.apiKeyId ?? "all",
      normalized.timezone || "local",
      normalized.startDate || "none",
      normalized.endDate || "none",
    ] as const;
  },
  usageLogs: (
    endpoint: string | null | undefined,
    sessionKey: string | null | undefined,
    query: Sub2APIUsageLogsQuery,
  ) => {
    const normalized = normalizeUsageLogsQuery(query);
    return [
      CLOUD_SERVICE_QUERY_ROOT,
      normalizeSessionKey(sessionKey),
      normalizeEndpointKey(endpoint),
      "usageLogs",
      normalized.page,
      normalized.pageSize,
      normalized.apiKeyId ?? "all",
      normalized.startDate || "none",
      normalized.endDate || "none",
    ] as const;
  },
  models: (endpoint: string | null | undefined, sessionKey: string | null | undefined) =>
    [
      CLOUD_SERVICE_QUERY_ROOT,
      normalizeSessionKey(sessionKey),
      normalizeEndpointKey(endpoint),
      "models",
    ] as const,
  paymentConfig: (
    endpoint: string | null | undefined,
    sessionKey: string | null | undefined,
    userId: number | null | undefined,
    locale: string | null | undefined,
  ) =>
    [
      CLOUD_SERVICE_QUERY_ROOT,
      normalizeSessionKey(sessionKey),
      normalizeEndpointKey(endpoint),
      "paymentConfig",
      userId ?? "none",
      normalizeSub2APILocale(locale),
    ] as const,
  groupStatuses: (endpoint: string | null | undefined, sessionKey: string | null | undefined) =>
    [
      CLOUD_SERVICE_QUERY_ROOT,
      normalizeSessionKey(sessionKey),
      normalizeEndpointKey(endpoint),
      "groupStatuses",
    ] as const,
  groupStatusHistory: (
    endpoint: string | null | undefined,
    sessionKey: string | null | undefined,
    groupId: number | null | undefined,
    period: Sub2APIGroupStatusHistoryPeriod,
  ) =>
    [
      CLOUD_SERVICE_QUERY_ROOT,
      normalizeSessionKey(sessionKey),
      normalizeEndpointKey(endpoint),
      "groupStatusHistory",
      groupId ?? "none",
      period,
    ] as const,
  groupStatusRecords: (
    endpoint: string | null | undefined,
    sessionKey: string | null | undefined,
    groupId: number | null | undefined,
    limit: number,
  ) =>
    [
      CLOUD_SERVICE_QUERY_ROOT,
      normalizeSessionKey(sessionKey),
      normalizeEndpointKey(endpoint),
      "groupStatusRecords",
      groupId ?? "none",
      limit,
    ] as const,
  groupStatusEvents: (
    endpoint: string | null | undefined,
    sessionKey: string | null | undefined,
    groupId: number | null | undefined,
    limit: number,
  ) =>
    [
      CLOUD_SERVICE_QUERY_ROOT,
      normalizeSessionKey(sessionKey),
      normalizeEndpointKey(endpoint),
      "groupStatusEvents",
      groupId ?? "none",
      limit,
    ] as const,
  referralInfo: (endpoint: string | null | undefined, sessionKey: string | null | undefined) =>
    [
      CLOUD_SERVICE_QUERY_ROOT,
      normalizeSessionKey(sessionKey),
      normalizeEndpointKey(endpoint),
      "referralInfo",
    ] as const,
  referralHistory: (endpoint: string | null | undefined, sessionKey: string | null | undefined) =>
    [
      CLOUD_SERVICE_QUERY_ROOT,
      normalizeSessionKey(sessionKey),
      normalizeEndpointKey(endpoint),
      "referralHistory",
    ] as const,
};

export function useSub2APIClient(): {
  client: Sub2APIClient | null;
  endpoint: string | null;
  sessionKey: string | null;
  isReady: boolean;
  isLoggedIn: boolean;
} {
  const { auth, isLoggedIn, getAccessToken } = useSub2APIAuth();
  const endpoint = auth?.endpoint ?? null;
  const sessionKey = auth?.sessionKey ?? null;

  const client = useMemo(() => {
    if (!isLoggedIn || !endpoint) {
      return null;
    }
    return createSub2APIClient({
      endpoint,
      getAccessToken,
    });
  }, [endpoint, getAccessToken, isLoggedIn]);

  return {
    client,
    endpoint,
    sessionKey,
    isReady: Boolean(client && isLoggedIn),
    isLoggedIn,
  };
}

export function useSub2APIMe() {
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();
  return useQuery({
    queryKey: cloudServiceQueryKeys.me(endpoint, sessionKey),
    enabled: isReady && client !== null,
    staleTime: 30_000,
    queryFn: async (): Promise<Sub2APIUser> => {
      if (!client) {
        throw new Error("Service client is unavailable.");
      }
      return await client.getMe();
    },
  });
}

export function useSub2APIKeys(page = 1, pageSize = 50) {
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();
  return useQuery({
    queryKey: [...cloudServiceQueryKeys.keys(endpoint, sessionKey), page, pageSize] as const,
    enabled: isReady && client !== null,
    staleTime: 15_000,
    queryFn: async (): Promise<Sub2APIPaginatedData<Sub2APIKey>> => {
      if (!client) {
        throw new Error("Service client is unavailable.");
      }
      return await client.listKeys(page, pageSize);
    },
  });
}

export function useSub2APIAvailableGroups() {
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();
  return useQuery({
    queryKey: cloudServiceQueryKeys.groups(endpoint, sessionKey),
    enabled: isReady && client !== null,
    staleTime: 60_000,
    queryFn: async (): Promise<Sub2APIGroup[]> => {
      if (!client) {
        throw new Error("Service client is unavailable.");
      }
      return await client.getAvailableGroups();
    },
  });
}

export function useSub2APIUsageStats(
  query: Sub2APIUsagePeriod | Sub2APIUsageStatsQuery,
  options?: { enabled?: boolean },
) {
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();
  const normalizedQuery = useMemo(() => normalizeUsageQuery(query), [query]);
  return useQuery({
    queryKey: cloudServiceQueryKeys.usage(endpoint, sessionKey, normalizedQuery),
    enabled: (options?.enabled ?? true) && isReady && client !== null,
    staleTime: 20_000,
    queryFn: async (): Promise<Sub2APIUsageStats> => {
      if (!client) {
        throw new Error("Service client is unavailable.");
      }
      return await client.getUsageStats(normalizedQuery);
    },
  });
}

export function useSub2APIUsageLogs(query: Sub2APIUsageLogsQuery, options?: { enabled?: boolean }) {
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();
  const normalizedQuery = useMemo(() => normalizeUsageLogsQuery(query), [query]);
  return useQuery({
    queryKey: cloudServiceQueryKeys.usageLogs(endpoint, sessionKey, normalizedQuery),
    enabled: (options?.enabled ?? true) && isReady && client !== null,
    staleTime: 15_000,
    queryFn: async (): Promise<Sub2APIPaginatedData<Sub2APIUsageLog>> => {
      if (!client) {
        throw new Error("Service client is unavailable.");
      }
      return await client.listUsageLogs(normalizedQuery);
    },
  });
}

export function useSub2APIModelCatalog() {
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();
  return useQuery({
    queryKey: cloudServiceQueryKeys.models(endpoint, sessionKey),
    enabled: isReady && client !== null,
    staleTime: 60_000,
    queryFn: async (): Promise<Sub2APIModelCatalog> => {
      if (!client) {
        throw new Error("Service client is unavailable.");
      }
      return await client.getModelCatalog();
    },
  });
}

function parsePositiveNumber(value: unknown): number | null {
  const numericValue =
    typeof value === "number" ? value : typeof value === "string" ? Number(value) : NaN;
  return Number.isFinite(numericValue) && numericValue > 0 ? numericValue : null;
}

function buildPayCenterUserConfigUrl(input: {
  endpoint: string;
  userId: number;
  accessToken: string;
  locale: string;
}): string {
  const base = normalizeSub2APIEndpoint(input.endpoint);
  const url = new URL(`${base}/pay/api/user`);
  url.searchParams.set("user_id", String(input.userId));
  url.searchParams.set("token", input.accessToken);
  url.searchParams.set("lang", normalizeSub2APILocale(input.locale));
  return url.toString();
}

export function useSub2APIPaymentConfig() {
  const { auth, getAccessToken, isLoggedIn } = useSub2APIAuth();
  const meQuery = useSub2APIMe();
  const endpoint = auth?.endpoint ?? null;
  const sessionKey = auth?.sessionKey ?? null;
  const userId = meQuery.data?.id ?? null;
  const locale = useSub2APILocale();

  return useQuery({
    queryKey: cloudServiceQueryKeys.paymentConfig(endpoint, sessionKey, userId, locale),
    enabled: isLoggedIn && Boolean(endpoint) && typeof userId === "number",
    staleTime: 5 * 60_000,
    queryFn: async (): Promise<Sub2APIPaymentConfig> => {
      if (!endpoint || typeof userId !== "number") {
        return { balanceCreditCnyPerUsd: null, usdExchangeRate: null };
      }
      const accessToken = await getAccessToken();
      if (!accessToken) {
        return { balanceCreditCnyPerUsd: null, usdExchangeRate: null };
      }

      try {
        const response = await fetch(
          buildPayCenterUserConfigUrl({ endpoint, userId, accessToken, locale }),
          {
            method: "GET",
            headers: { "Accept-Language": normalizeSub2APILocale(locale) },
          },
        );
        if (!response.ok) {
          return { balanceCreditCnyPerUsd: null, usdExchangeRate: null };
        }
        const payload = (await response.json()) as {
          config?: {
            balanceCreditCnyPerUsd?: unknown;
            usdExchangeRate?: unknown;
          };
        };
        return {
          balanceCreditCnyPerUsd: parsePositiveNumber(payload.config?.balanceCreditCnyPerUsd),
          usdExchangeRate: parsePositiveNumber(payload.config?.usdExchangeRate),
        };
      } catch {
        return { balanceCreditCnyPerUsd: null, usdExchangeRate: null };
      }
    },
  });
}

export function useCreateSub2APIKeyMutation() {
  const queryClient = useQueryClient();
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();

  return useMutation({
    mutationFn: async (input: Sub2APICreateKeyRequest): Promise<Sub2APIKey> => {
      if (!client || !isReady) {
        throw new Error("Service client is unavailable.");
      }
      return await client.createKey(input);
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: cloudServiceQueryKeys.keys(endpoint, sessionKey),
        }),
        queryClient.invalidateQueries({
          queryKey: cloudServiceQueryKeys.me(endpoint, sessionKey),
        }),
      ]);
    },
  });
}

export function useDeleteSub2APIKeyMutation() {
  const queryClient = useQueryClient();
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();

  return useMutation({
    mutationFn: async (id: number): Promise<void> => {
      if (!client || !isReady) {
        throw new Error("Service client is unavailable.");
      }
      await client.deleteKey(id);
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: cloudServiceQueryKeys.keys(endpoint, sessionKey),
        }),
        queryClient.invalidateQueries({
          queryKey: cloudServiceQueryKeys.me(endpoint, sessionKey),
        }),
      ]);
    },
  });
}

export function useUpdateSub2APIKeyMutation() {
  const queryClient = useQueryClient();
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();

  return useMutation({
    mutationFn: async (input: {
      id: number;
      patch: Sub2APIUpdateKeyRequest;
    }): Promise<Sub2APIKey> => {
      if (!client || !isReady) {
        throw new Error("Service client is unavailable.");
      }
      return await client.updateKey(input.id, input.patch);
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: cloudServiceQueryKeys.keys(endpoint, sessionKey),
        }),
        queryClient.invalidateQueries({
          queryKey: cloudServiceQueryKeys.me(endpoint, sessionKey),
        }),
      ]);
    },
  });
}

export function useSub2APIGroupStatuses() {
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();
  return useQuery({
    queryKey: cloudServiceQueryKeys.groupStatuses(endpoint, sessionKey),
    enabled: isReady && client !== null,
    staleTime: 30_000,
    queryFn: async (): Promise<Sub2APIGroupStatusItem[]> => {
      if (!client) {
        throw new Error("Service client is unavailable.");
      }
      return await client.getGroupStatuses();
    },
  });
}

export function useSub2APIGroupStatusHistory(
  groupId: number | null,
  period: Sub2APIGroupStatusHistoryPeriod,
  options?: { enabled?: boolean },
) {
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();
  return useQuery({
    queryKey: cloudServiceQueryKeys.groupStatusHistory(endpoint, sessionKey, groupId, period),
    enabled: (options?.enabled ?? true) && groupId !== null && isReady && client !== null,
    staleTime: 30_000,
    queryFn: async (): Promise<Sub2APIGroupStatusHistoryBucket[]> => {
      if (!client || groupId === null) {
        throw new Error("Service client is unavailable.");
      }
      return await client.getGroupStatusHistory(groupId, period);
    },
  });
}

export function useSub2APIGroupStatusRecords(
  groupId: number | null,
  limit = 24,
  options?: { enabled?: boolean },
) {
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();
  return useQuery({
    queryKey: cloudServiceQueryKeys.groupStatusRecords(endpoint, sessionKey, groupId, limit),
    enabled: (options?.enabled ?? true) && groupId !== null && isReady && client !== null,
    staleTime: 30_000,
    queryFn: async (): Promise<Sub2APIGroupStatusRecord[]> => {
      if (!client || groupId === null) {
        throw new Error("Service client is unavailable.");
      }
      return await client.getGroupStatusRecords(groupId, limit);
    },
  });
}

export function useSub2APIGroupStatusEvents(
  groupId: number | null,
  limit = 20,
  options?: { enabled?: boolean },
) {
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();
  return useQuery({
    queryKey: cloudServiceQueryKeys.groupStatusEvents(endpoint, sessionKey, groupId, limit),
    enabled: (options?.enabled ?? true) && groupId !== null && isReady && client !== null,
    staleTime: 30_000,
    queryFn: async (): Promise<Sub2APIGroupStatusEvent[]> => {
      if (!client || groupId === null) {
        throw new Error("Service client is unavailable.");
      }
      return await client.getGroupStatusEvents(groupId, limit);
    },
  });
}

export function useSub2APIReferralInfo() {
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();
  return useQuery({
    queryKey: cloudServiceQueryKeys.referralInfo(endpoint, sessionKey),
    enabled: isReady && client !== null,
    staleTime: 60_000,
    queryFn: async (): Promise<Sub2APIReferralInfo> => {
      if (!client) {
        throw new Error("Service client is unavailable.");
      }
      return await client.getReferralInfo();
    },
  });
}

export function useSub2APIReferralHistory(page = 1, pageSize = 20) {
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();
  return useQuery({
    queryKey: [
      ...cloudServiceQueryKeys.referralHistory(endpoint, sessionKey),
      page,
      pageSize,
    ] as const,
    enabled: isReady && client !== null,
    staleTime: 30_000,
    queryFn: async (): Promise<Sub2APIPaginatedData<Sub2APIUserReferral>> => {
      if (!client) {
        throw new Error("Service client is unavailable.");
      }
      return await client.getReferralHistory(page, pageSize);
    },
  });
}

export function useRedeemCodeMutation() {
  const queryClient = useQueryClient();
  const { client, endpoint, sessionKey, isReady } = useSub2APIClient();

  return useMutation({
    mutationFn: async (code: string): Promise<Sub2APIRedeemResult> => {
      if (!client || !isReady) {
        throw new Error("Service client is unavailable.");
      }
      return await client.redeemCode(code);
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: cloudServiceQueryKeys.me(endpoint, sessionKey),
        }),
        queryClient.invalidateQueries({
          queryKey: cloudServiceQueryKeys.referralInfo(endpoint, sessionKey),
        }),
      ]);
    },
  });
}
