import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { Alert } from "react-native";
import { invokeDesktopCommand } from "@/desktop/electron/invoke";
import { getIsElectron } from "@/constants/platform";
import { useSub2APIAuth } from "@/hooks/use-sub2api-auth";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";
import type {
  DesktopProviderPayload,
  ManagedProviderTarget,
  ProviderStore,
} from "@/screens/settings/sub2api-provider-types";
import { isValidSub2APIEndpoint } from "./sub2api-auth-bridge";
import { getErrorMessage } from "./managed-provider-settings-shared";
import {
  defaultModelsForTarget,
  fetchGatewayModelIds,
  filterGatewayModelsForTarget,
  targetNeedsModelList,
} from "./gateway-models";

export type DesktopProvidersStoreValue = {
  providers: DesktopProviderPayload[];
  /** Active row per target; the two named fields below are the Claude/Codex entries of this. */
  activeProviderIds: Record<ManagedProviderTarget, string | null>;
  activeClaudeProviderId: string | null;
  activeCodexProviderId: string | null;
  activeClaudeProvider: DesktopProviderPayload | null;
  activeCodexProvider: DesktopProviderPayload | null;
  /** API key strings (trimmed) in use on disk for Claude or Codex. */
  activeRouteApiKeys: string[];
  loadProviders: () => Promise<void>;
  showAddProviderForm: boolean;
  editProviderName: string;
  setEditProviderName: (s: string) => void;
  editProviderEndpoint: string;
  setEditProviderEndpoint: (s: string) => void;
  editProviderApiKey: string;
  setEditProviderApiKey: (s: string) => void;
  customTarget: ManagedProviderTarget;
  setCustomTarget: (t: ManagedProviderTarget) => void;
  openCustomProviderForm: () => void;
  closeCustomProviderForm: () => void;
  handleSwitchProvider: (id: string, scope?: ManagedProviderTarget) => Promise<void>;
  handleRemoveProvider: (id: string) => Promise<void>;
  handleAddProvider: () => Promise<void>;
};

const DesktopProvidersContext = createContext<DesktopProvidersStoreValue | null>(null);

export function resolveScopedActiveProviderIds(
  store: ProviderStore,
): Record<ManagedProviderTarget, string | null> {
  const hasScopedIds =
    store.activeClaudeProviderId !== null || store.activeCodexProviderId !== null;
  const legacyFallback = hasScopedIds ? null : (store.activeProviderId ?? null);
  return {
    claude: store.activeClaudeProviderId ?? legacyFallback,
    codex: store.activeCodexProviderId ?? legacyFallback,
    // Grok and Pi are opt-in targets, so a legacy unscoped row never implies them.
    grok: store.activeGrokProviderId ?? null,
    pi: store.activePiProviderId ?? null,
  };
}

const EMPTY_ACTIVE_IDS: Record<ManagedProviderTarget, string | null> = {
  claude: null,
  codex: null,
  grok: null,
  pi: null,
};

export function DesktopProvidersStoreProvider({ children }: { children: ReactNode }) {
  const { auth, isLoggedIn } = useSub2APIAuth();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.desktopProviders, [locale]);
  const isElectron = getIsElectron();
  const [providers, setProviders] = useState<DesktopProviderPayload[]>([]);
  const [activeProviderIds, setActiveProviderIds] =
    useState<Record<ManagedProviderTarget, string | null>>(EMPTY_ACTIVE_IDS);
  const [showAddProviderForm, setShowAddProviderForm] = useState(false);
  const [editProviderName, setEditProviderName] = useState("");
  const [editProviderEndpoint, setEditProviderEndpoint] = useState("");
  const [editProviderApiKey, setEditProviderApiKey] = useState("");
  const [customTarget, setCustomTarget] = useState<ManagedProviderTarget>("claude");

  const loadProviders = useCallback(async () => {
    if (!isElectron) {
      return;
    }
    try {
      const store = await invokeDesktopCommand<ProviderStore>("get_providers");
      setProviders(store.providers);
      setActiveProviderIds(resolveScopedActiveProviderIds(store));
    } catch {
      setProviders([]);
      setActiveProviderIds(EMPTY_ACTIVE_IDS);
    }
  }, [isElectron]);

  useEffect(() => {
    void loadProviders();
  }, [loadProviders]);

  // After managed-service OAuth (handled at app root), refresh provider list so Claude/Codex routes match the new account.
  useEffect(() => {
    if (!isLoggedIn) {
      return;
    }
    void loadProviders();
  }, [auth?.endpoint, auth?.sessionKey, isLoggedIn, loadProviders]);

  const openCustomProviderForm = useCallback(() => {
    setShowAddProviderForm(true);
    setEditProviderName("");
    setEditProviderEndpoint("");
    setEditProviderApiKey("");
    setCustomTarget("claude");
  }, []);

  const closeCustomProviderForm = useCallback(() => {
    setShowAddProviderForm(false);
    setEditProviderName("");
    setEditProviderEndpoint("");
    setEditProviderApiKey("");
    setCustomTarget("claude");
  }, []);

  const handleSwitchProvider = useCallback(
    async (id: string, scope?: ManagedProviderTarget) => {
      try {
        // Grok and Pi embed an explicit model list in their config, so it is read from the
        // gateway first. A BYOK endpoint need not serve /v1/models, so an unreachable catalog
        // falls back to defaults rather than blocking the write — the config file is what the
        // user edits by hand afterwards.
        let models: Record<string, string[]> = {};
        let usedFallback = false;
        if (scope && targetNeedsModelList(scope)) {
          const provider = providers.find((entry) => entry.id === id);
          if (!provider) {
            throw new Error(text.providerRowMissing);
          }
          let catalog: string[] = [];
          try {
            catalog = await fetchGatewayModelIds({
              endpoint: provider.endpoint,
              apiKey: provider.apiKey,
            });
          } catch {
            catalog = [];
          }
          const usable = filterGatewayModelsForTarget(scope, catalog);
          usedFallback = usable.length === 0;
          const resolved = usedFallback ? defaultModelsForTarget(scope) : usable;
          models = scope === "grok" ? { grokModels: resolved } : { piModels: resolved };
        }

        await invokeDesktopCommand("switch_provider", {
          id,
          ...(scope ? { scope } : {}),
          ...models,
        });
        await loadProviders();
        if (usedFallback) {
          Alert.alert(text.catalogUnavailableTitle, text.catalogUnavailableBody);
        }
      } catch (error) {
        Alert.alert(text.switchFailed, getErrorMessage(error));
      }
    },
    [
      loadProviders,
      providers,
      text.catalogUnavailableBody,
      text.catalogUnavailableTitle,
      text.providerRowMissing,
      text.switchFailed,
    ],
  );

  const handleAddProvider = useCallback(async () => {
    const name = editProviderName.trim();
    const endpoint = editProviderEndpoint.trim().replace(/\/+$/, "");
    const apiKey = editProviderApiKey.trim();

    if (!name || !endpoint || !apiKey) {
      Alert.alert(text.missingInformationTitle, text.missingInformationBody);
      return;
    }
    if (!isValidSub2APIEndpoint(endpoint)) {
      Alert.alert(text.invalidEndpointTitle, text.invalidEndpointBody);
      return;
    }

    const provider: DesktopProviderPayload = {
      id: `custom-${Date.now()}`,
      name,
      type: "custom",
      endpoint,
      apiKey,
      isDefault: false,
      target: customTarget,
      // These two fields are Claude/Codex-specific wire settings; Grok and Pi carry neither.
      ...(customTarget === "claude" ? { claudeApiFormat: "anthropic" as const } : {}),
      ...(customTarget === "codex" ? { codexWireApi: "responses" as const } : {}),
    };

    try {
      await invokeDesktopCommand("add_provider", provider as unknown as Record<string, unknown>);
      closeCustomProviderForm();
      await loadProviders();
    } catch (error) {
      Alert.alert(text.addProviderFailed, getErrorMessage(error));
    }
  }, [
    closeCustomProviderForm,
    customTarget,
    editProviderApiKey,
    editProviderEndpoint,
    editProviderName,
    loadProviders,
    text.addProviderFailed,
    text.invalidEndpointBody,
    text.invalidEndpointTitle,
    text.missingInformationBody,
    text.missingInformationTitle,
  ]);

  const handleRemoveProvider = useCallback(
    async (id: string) => {
      try {
        await invokeDesktopCommand("remove_provider", { id });
        await loadProviders();
      } catch (error) {
        Alert.alert(text.removeProviderFailed, getErrorMessage(error));
      }
    },
    [loadProviders, text.removeProviderFailed],
  );

  const activeClaudeProviderId = activeProviderIds.claude;
  const activeCodexProviderId = activeProviderIds.codex;

  const activeClaudeProvider = useMemo(
    () => providers.find((p) => p.id === activeClaudeProviderId) ?? null,
    [providers, activeClaudeProviderId],
  );
  const activeCodexProvider = useMemo(
    () => providers.find((p) => p.id === activeCodexProviderId) ?? null,
    [providers, activeCodexProviderId],
  );

  const activeRouteApiKeys = useMemo(() => {
    const keys = new Set<string>();
    const a = activeClaudeProvider?.apiKey?.trim();
    const b = activeCodexProvider?.apiKey?.trim();
    if (a) keys.add(a);
    if (b) keys.add(b);
    return [...keys];
  }, [activeClaudeProvider, activeCodexProvider]);

  const value = useMemo(
    (): DesktopProvidersStoreValue => ({
      providers,
      activeProviderIds,
      activeClaudeProviderId,
      activeCodexProviderId,
      activeClaudeProvider,
      activeCodexProvider,
      activeRouteApiKeys,
      loadProviders,
      showAddProviderForm,
      editProviderName,
      setEditProviderName,
      editProviderEndpoint,
      setEditProviderEndpoint,
      editProviderApiKey,
      setEditProviderApiKey,
      customTarget,
      setCustomTarget,
      openCustomProviderForm,
      closeCustomProviderForm,
      handleSwitchProvider,
      handleRemoveProvider,
      handleAddProvider,
    }),
    [
      activeClaudeProvider,
      activeClaudeProviderId,
      activeCodexProvider,
      activeCodexProviderId,
      activeProviderIds,
      activeRouteApiKeys,
      closeCustomProviderForm,
      customTarget,
      editProviderApiKey,
      editProviderEndpoint,
      editProviderName,
      handleAddProvider,
      handleRemoveProvider,
      handleSwitchProvider,
      loadProviders,
      openCustomProviderForm,
      providers,
      showAddProviderForm,
    ],
  );

  return (
    <DesktopProvidersContext.Provider value={value}>{children}</DesktopProvidersContext.Provider>
  );
}

export function useDesktopProvidersStore(): DesktopProvidersStoreValue {
  const ctx = useContext(DesktopProvidersContext);
  if (!ctx) {
    throw new Error("useDesktopProvidersStore must be used within DesktopProvidersStoreProvider");
  }
  return ctx;
}
