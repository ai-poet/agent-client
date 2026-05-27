import { useEffect, useSyncExternalStore } from "react";
import { usePathname, useRouter } from "expo-router";
import { StartupSplashScreen } from "@/screens/startup-splash-screen";
import { useHostRuntimeBootstrapState, useStoreReady } from "@/app/_layout";
import { getHostRuntimeStore, isHostRuntimeConnected, useHosts } from "@/runtime/host-runtime";
import { buildHostRootRoute } from "@/utils/host-routes";
import { shouldUseDesktopDaemon } from "@/desktop/daemon/desktop-daemon";
import { useAppSettings } from "@/hooks/use-settings";
import { useSub2APIAuth } from "@/hooks/use-sub2api-auth";

const WELCOME_ROUTE = "/welcome" as const;
const LOGIN_ROUTE = "/login" as const;
const MODE_SELECT_ROUTE = "/mode-select" as const;
const SETUP_CHECK_ROUTE = "/setup-check" as const;

function useAnyOnlineHostServerId(serverIds: string[]): string | null {
  const runtime = getHostRuntimeStore();

  return useSyncExternalStore(
    (onStoreChange) => runtime.subscribeAll(onStoreChange),
    () => {
      let firstOnlineServerId: string | null = null;
      let firstOnlineAt: string | null = null;
      for (const serverId of serverIds) {
        const snapshot = runtime.getSnapshot(serverId);
        const lastOnlineAt = snapshot?.lastOnlineAt ?? null;
        if (!isHostRuntimeConnected(snapshot) || !lastOnlineAt) {
          continue;
        }
        if (!firstOnlineAt || lastOnlineAt < firstOnlineAt) {
          firstOnlineAt = lastOnlineAt;
          firstOnlineServerId = serverId;
        }
      }
      return firstOnlineServerId;
    },
    () => null,
  );
}

const isDesktop = shouldUseDesktopDaemon();

export default function Index() {
  const router = useRouter();
  const pathname = usePathname();
  const bootstrapState = useHostRuntimeBootstrapState();
  const storeReady = useStoreReady();
  const hosts = useHosts();
  const anyOnlineServerId = useAnyOnlineHostServerId(hosts.map((host) => host.serverId));
  const { settings, isLoading: settingsLoading } = useAppSettings();
  const { isLoggedIn, isLoading: authLoading } = useSub2APIAuth();

  useEffect(() => {
    if (pathname !== "/" && pathname !== "") {
      return;
    }
    if (settingsLoading || authLoading) {
      return;
    }

    if (settings.experienceMode === null) {
      router.replace(MODE_SELECT_ROUTE);
      return;
    }
    if (settings.accessMode === null) {
      router.replace(MODE_SELECT_ROUTE);
      return;
    }
    if (settings.accessMode === "builtin" && !isLoggedIn) {
      router.replace(LOGIN_ROUTE);
      return;
    }

    // Redirect to setup check if not yet completed
    if (!settings.setupCheckCompleted) {
      router.replace(SETUP_CHECK_ROUTE);
      return;
    }

    if (!storeReady) {
      return;
    }

    const targetRoute = anyOnlineServerId ? buildHostRootRoute(anyOnlineServerId) : WELCOME_ROUTE;
    router.replace(targetRoute);
  }, [
    anyOnlineServerId,
    authLoading,
    isLoggedIn,
    pathname,
    router,
    settings.accessMode,
    settings.experienceMode,
    settings.setupCheckCompleted,
    settingsLoading,
    storeReady,
  ]);

  return <StartupSplashScreen bootstrapState={isDesktop ? bootstrapState : undefined} />;
}
