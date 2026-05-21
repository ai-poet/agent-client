import AsyncStorage from "@react-native-async-storage/async-storage";
import { queryClient } from "@/query/query-client";
import { getHostRuntimeStore } from "@/runtime/host-runtime";
import { invokeDesktopCommand } from "@/desktop/electron/invoke";
import { isWeb } from "@/constants/platform";

const PASEO_STORAGE_KEYS = [
  "panel-state",
  "workspace-tabs-state",
  "workspace-layout-state",
  "sidebar-project-names",
  "sidebar-sort",
  "section-order",
  "sidebar-project-workspace-order",
  "sidebar-collapsed-sections",
  "paseo-drafts",
];

const PASEO_ATTACHMENT_DB_NAME = "paseo-attachment-bytes";

async function removePaseoAsyncStorage(): Promise<void> {
  const keys = await AsyncStorage.getAllKeys();
  const keysToRemove = keys.filter(
    (key) => key.startsWith("@paseo:") || PASEO_STORAGE_KEYS.includes(key),
  );
  if (keysToRemove.length > 0) {
    await AsyncStorage.multiRemove(keysToRemove);
  }
}

function deleteIndexedDb(name: string): Promise<void> {
  return new Promise((resolve) => {
    const idb = globalThis.indexedDB;
    if (!idb) {
      resolve();
      return;
    }
    const request = idb.deleteDatabase(name);
    request.onsuccess = () => resolve();
    request.onerror = () => resolve();
    request.onblocked = () => resolve();
  });
}

async function removePaseoIndexedDb(): Promise<void> {
  if (!isWeb || typeof indexedDB === "undefined") {
    return;
  }
  await deleteIndexedDb(PASEO_ATTACHMENT_DB_NAME);
}

function reloadApp(): void {
  if (isWeb && typeof window !== "undefined") {
    window.location.assign("/");
  }
}

export async function resetPaseoEnvironment(): Promise<void> {
  await invokeDesktopCommand("reset_paseo_environment");
  await getHostRuntimeStore().reset();
  queryClient.clear();
  await Promise.all([removePaseoAsyncStorage(), removePaseoIndexedDb()]);
  reloadApp();
}

export const __private__ = {
  PASEO_STORAGE_KEYS,
  PASEO_ATTACHMENT_DB_NAME,
  removePaseoAsyncStorage,
  removePaseoIndexedDb,
};
