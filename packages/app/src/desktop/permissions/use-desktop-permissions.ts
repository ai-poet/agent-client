import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  DEFAULT_DESKTOP_PERMISSION_MESSAGES,
  getDesktopPermissionSnapshot,
  requestDesktopPermission,
  shouldShowDesktopPermissionSection,
  type DesktopPermissionKind,
  type DesktopPermissionMessages,
  type DesktopPermissionSnapshot,
} from "@/desktop/permissions/desktop-permissions";
import { sendOsNotification } from "@/utils/os-notifications";
import { APP_NAME } from "@/config/branding";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";
import { getSub2APIMessages } from "@/i18n/sub2api";

export interface UseDesktopPermissionsReturn {
  isDesktopApp: boolean;
  snapshot: DesktopPermissionSnapshot | null;
  isRefreshing: boolean;
  requestingPermission: DesktopPermissionKind | null;
  isSendingTestNotification: boolean;
  testNotificationError: string | null;
  refreshPermissions: () => Promise<void>;
  requestPermission: (kind: DesktopPermissionKind) => Promise<void>;
  sendTestNotification: () => Promise<void>;
}

export function useDesktopPermissions(): UseDesktopPermissionsReturn {
  const isDesktopApp = shouldShowDesktopPermissionSection();
  const locale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(locale).settings.permissions, [locale]);
  const permissionMessages = useMemo<DesktopPermissionMessages>(
    () => ({
      ...DEFAULT_DESKTOP_PERMISSION_MESSAGES,
      notifications: {
        ...DEFAULT_DESKTOP_PERMISSION_MESSAGES.notifications,
        allowedByOs: text.details.notifications.granted,
        deniedInSystem: text.details.notifications.denied,
        notGrantedYet: text.details.notifications.prompt,
        desktopStatusWebOnly: text.details.notifications.unavailable,
        webApiUnavailable: text.details.notifications.unavailable,
        requestWebOnly: text.details.notifications.unavailable,
        requestApiUnavailable: text.details.notifications.unavailable,
      },
      microphone: {
        ...DEFAULT_DESKTOP_PERMISSION_MESSAGES.microphone,
        desktopStatusWebOnly: text.details.microphone.unavailable,
        navigatorUnavailable: text.details.microphone.unavailable,
        granted: text.details.microphone.granted,
        deniedInSystem: text.details.microphone.denied,
        notGrantedYet: text.details.microphone.prompt,
        captureUnavailable: text.details.microphone.unavailable,
        permissionStatusUnavailable: text.details.microphone.unknown,
        requestWebOnly: text.details.microphone.unavailable,
        requestCaptureUnavailable: text.details.microphone.unavailable,
      },
    }),
    [text.details.microphone, text.details.notifications],
  );
  const isMountedRef = useRef(true);
  const [snapshot, setSnapshot] = useState<DesktopPermissionSnapshot | null>(null);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [requestingPermission, setRequestingPermission] = useState<DesktopPermissionKind | null>(
    null,
  );
  const [isSendingTestNotification, setIsSendingTestNotification] = useState(false);

  useEffect(() => {
    return () => {
      isMountedRef.current = false;
    };
  }, []);

  const refreshPermissions = useCallback(async () => {
    if (!isDesktopApp) {
      return;
    }

    setIsRefreshing(true);
    try {
      const nextSnapshot = await getDesktopPermissionSnapshot(permissionMessages);
      if (!isMountedRef.current) {
        return;
      }
      setSnapshot(nextSnapshot);
    } catch (error) {
      console.error("[Settings] Failed to load desktop permission status", error);
    } finally {
      if (isMountedRef.current) {
        setIsRefreshing(false);
      }
    }
  }, [isDesktopApp, permissionMessages]);

  const requestPermission = useCallback(
    async (kind: DesktopPermissionKind) => {
      if (!isDesktopApp) {
        return;
      }

      setRequestingPermission(kind);
      try {
        const status = await requestDesktopPermission({ kind, messages: permissionMessages });
        if (!isMountedRef.current) {
          return;
        }

        setSnapshot((previous) => {
          const base: DesktopPermissionSnapshot = previous ?? {
            checkedAt: Date.now(),
            notifications: {
              state: "unknown",
              detail: text.details.notifications.unknown,
            },
            microphone: {
              state: "unknown",
              detail: text.details.microphone.unknown,
            },
          };

          if (kind === "notifications") {
            return {
              ...base,
              checkedAt: Date.now(),
              notifications: status,
            };
          }

          return {
            ...base,
            checkedAt: Date.now(),
            microphone: status,
          };
        });
      } catch (error) {
        console.error(`[Settings] Failed to request ${kind} permission`, error);
      } finally {
        if (isMountedRef.current) {
          setRequestingPermission(null);
        }
        await refreshPermissions();
      }
    },
    [
      isDesktopApp,
      permissionMessages,
      refreshPermissions,
      text.details.microphone.unknown,
      text.details.notifications.unknown,
    ],
  );

  const [testNotificationError, setTestNotificationError] = useState<string | null>(null);

  const sendTestNotification = useCallback(async () => {
    if (!isDesktopApp) {
      return;
    }

    setIsSendingTestNotification(true);
    setTestNotificationError(null);
    try {
      const sent = await sendOsNotification({
        title: text.testNotificationTitle(APP_NAME),
        body: text.testNotificationBody,
      });
      if (!sent) {
        setTestNotificationError(text.notificationNotDelivered);
      }
    } catch (error) {
      setTestNotificationError(text.failedSendNotification);
    } finally {
      if (isMountedRef.current) {
        setIsSendingTestNotification(false);
      }
    }
  }, [
    isDesktopApp,
    text.failedSendNotification,
    text.notificationNotDelivered,
    text.testNotificationBody,
    text.testNotificationTitle,
  ]);

  useEffect(() => {
    if (!isDesktopApp) {
      return;
    }

    void refreshPermissions();
  }, [isDesktopApp, refreshPermissions]);

  return {
    isDesktopApp,
    snapshot,
    isRefreshing,
    requestingPermission,
    isSendingTestNotification,
    testNotificationError,
    refreshPermissions,
    requestPermission,
    sendTestNotification,
  };
}
