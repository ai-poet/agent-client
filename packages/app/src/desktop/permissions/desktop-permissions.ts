import { getDesktopHost } from "@/desktop/host";
import { isWeb, isNative } from "@/constants/platform";

export type DesktopPermissionKind = "notifications" | "microphone";

export type DesktopPermissionState =
  | "granted"
  | "denied"
  | "prompt"
  | "not-granted"
  | "unavailable"
  | "unknown";

export interface DesktopPermissionStatus {
  state: DesktopPermissionState;
  detail: string;
}

export interface DesktopPermissionSnapshot {
  checkedAt: number;
  notifications: DesktopPermissionStatus;
  microphone: DesktopPermissionStatus;
}

export interface DesktopPermissionMessages {
  notifications: {
    allowedByOs: string;
    deniedInSystem: string;
    notGrantedYet: string;
    unexpectedState: (permission: string) => string;
    desktopStatusWebOnly: string;
    supported: string;
    notSupported: string;
    webApiUnavailable: string;
    requestWebOnly: string;
    requestApiUnavailable: string;
    failedRequest: (message: string) => string;
  };
  microphone: {
    desktopStatusWebOnly: string;
    navigatorUnavailable: string;
    granted: string;
    deniedInSystem: string;
    notGrantedYet: string;
    unexpectedState: (state: string) => string;
    runtimeStatusApiUnavailable: string;
    failedQuery: (message: string) => string;
    captureUnavailable: string;
    permissionStatusUnavailable: string;
    requestWebOnly: string;
    requestCaptureUnavailable: string;
    deniedByUserOrSystem: string;
    noDeviceFound: string;
    failedRequest: (message: string) => string;
  };
}

type NotificationConstructorLike = {
  permission?: string;
  requestPermission?: () => Promise<string>;
};

type MediaStreamTrackLike = {
  stop?: () => void;
};

type MediaStreamLike = {
  getTracks?: () => MediaStreamTrackLike[];
};

type NavigatorLike = {
  mediaDevices?: {
    getUserMedia?: (constraints: { audio: boolean }) => Promise<MediaStreamLike>;
  };
  permissions?: {
    query?: (descriptor: { name: string }) => Promise<{ state?: string }>;
  };
};

export function shouldShowDesktopPermissionSection(): boolean {
  return isWeb && getDesktopHost() !== null;
}

function status(input: DesktopPermissionStatus): DesktopPermissionStatus {
  return input;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function getErrorMessage(error: unknown): string {
  if (error instanceof Error && typeof error.message === "string") {
    return error.message;
  }
  return String(error);
}

function getErrorName(error: unknown): string | null {
  if (!isObject(error)) {
    return null;
  }
  const name = error.name;
  return typeof name === "string" && name.length > 0 ? name : null;
}

function isPermissionsQueryRuntimeUnsupported(error: unknown): boolean {
  const message = getErrorMessage(error);
  if (
    message.includes("Can only call Permissions.query on instances of Permissions") ||
    message.includes("Illegal invocation")
  ) {
    return true;
  }
  return false;
}

function getWebNotificationConstructor(): NotificationConstructorLike | null {
  if (isNative) {
    return null;
  }
  const NotificationConstructor = (globalThis as { Notification?: unknown }).Notification;
  if (
    NotificationConstructor == null ||
    (typeof NotificationConstructor !== "function" && typeof NotificationConstructor !== "object")
  ) {
    return null;
  }
  return NotificationConstructor as NotificationConstructorLike;
}

function getNavigatorLike(): NavigatorLike | null {
  if (isNative) {
    return null;
  }
  const webNavigator = (globalThis as { navigator?: unknown }).navigator;
  if (!isObject(webNavigator)) {
    return null;
  }
  return webNavigator as NavigatorLike;
}

export const DEFAULT_DESKTOP_PERMISSION_MESSAGES: DesktopPermissionMessages = {
  notifications: {
    allowedByOs: "Notifications are allowed by the OS.",
    deniedInSystem: "Notifications are denied in system settings.",
    notGrantedYet: "Notifications have not been granted yet.",
    unexpectedState: (permission: string) =>
      `Unexpected notification permission state: ${permission}`,
    desktopStatusWebOnly: "Desktop notification status is only available on web runtime.",
    supported: "Desktop notifications are supported.",
    notSupported: "Desktop notifications are not supported on this platform.",
    webApiUnavailable: "Web Notification API is unavailable in this environment.",
    requestWebOnly: "Desktop notification requests are only available on web runtime.",
    requestApiUnavailable: "Web Notification API requestPermission() is unavailable.",
    failedRequest: (message: string) => `Failed to request notification permission: ${message}`,
  },
  microphone: {
    desktopStatusWebOnly: "Desktop microphone status is only available on web runtime.",
    navigatorUnavailable: "Navigator is unavailable in this environment.",
    granted: "Microphone access is granted.",
    deniedInSystem: "Microphone access is denied in system settings.",
    notGrantedYet: "Microphone permission has not been granted yet.",
    unexpectedState: (state: string) => `Unexpected microphone permission state: ${state}`,
    runtimeStatusApiUnavailable:
      "Microphone status API is unavailable in this runtime. Use Request to check access.",
    failedQuery: (message: string) => `Failed to query microphone status: ${message}`,
    captureUnavailable: "Microphone capture is unavailable in this environment.",
    permissionStatusUnavailable:
      "Permission status API is unavailable. Use Request to check access.",
    requestWebOnly: "Desktop microphone requests are only available on web runtime.",
    requestCaptureUnavailable: "Microphone capture API is unavailable in this environment.",
    deniedByUserOrSystem: "Microphone permission was denied by the user or system.",
    noDeviceFound: "No microphone device was found.",
    failedRequest: (message: string) => `Failed to request microphone permission: ${message}`,
  },
};

function mapNotificationPermissionString(
  permission: string,
  messages: DesktopPermissionMessages,
): DesktopPermissionStatus {
  if (permission === "granted") {
    return status({
      state: "granted",
      detail: messages.notifications.allowedByOs,
    });
  }
  if (permission === "denied") {
    return status({
      state: "denied",
      detail: messages.notifications.deniedInSystem,
    });
  }
  if (permission === "default") {
    return status({
      state: "prompt",
      detail: messages.notifications.notGrantedYet,
    });
  }
  return status({
    state: "unknown",
    detail: messages.notifications.unexpectedState(permission),
  });
}

async function getNotificationPermissionStatus(
  messages: DesktopPermissionMessages,
): Promise<DesktopPermissionStatus> {
  if (isNative) {
    return status({
      state: "unavailable",
      detail: messages.notifications.desktopStatusWebOnly,
    });
  }

  const desktopHost = getDesktopHost();
  if (desktopHost && typeof desktopHost.notification?.isSupported === "function") {
    try {
      const supported = await desktopHost.notification.isSupported();
      return status({
        state: supported ? "granted" : "unavailable",
        detail: supported ? messages.notifications.supported : messages.notifications.notSupported,
      });
    } catch {
      // Fall through to web API check
    }
  }

  const NotificationConstructor = getWebNotificationConstructor();
  if (NotificationConstructor && typeof NotificationConstructor.permission === "string") {
    return mapNotificationPermissionString(NotificationConstructor.permission, messages);
  }

  return status({
    state: "unavailable",
    detail: messages.notifications.webApiUnavailable,
  });
}

async function getMicrophonePermissionStatus(
  messages: DesktopPermissionMessages,
): Promise<DesktopPermissionStatus> {
  if (isNative) {
    return status({
      state: "unavailable",
      detail: messages.microphone.desktopStatusWebOnly,
    });
  }

  const webNavigator = getNavigatorLike();
  if (!webNavigator) {
    return status({
      state: "unavailable",
      detail: messages.microphone.navigatorUnavailable,
    });
  }

  const permissionsApi = webNavigator.permissions;
  if (permissionsApi && typeof permissionsApi.query === "function") {
    try {
      const result = await permissionsApi.query({ name: "microphone" });
      if (result?.state === "granted") {
        return status({
          state: "granted",
          detail: messages.microphone.granted,
        });
      }
      if (result?.state === "denied") {
        return status({
          state: "denied",
          detail: messages.microphone.deniedInSystem,
        });
      }
      if (result?.state === "prompt") {
        return status({
          state: "prompt",
          detail: messages.microphone.notGrantedYet,
        });
      }
      return status({
        state: "unknown",
        detail: messages.microphone.unexpectedState(result?.state ?? "unknown"),
      });
    } catch (error) {
      if (isPermissionsQueryRuntimeUnsupported(error)) {
        return status({
          state: "unknown",
          detail: messages.microphone.runtimeStatusApiUnavailable,
        });
      }
      return status({
        state: "unknown",
        detail: messages.microphone.failedQuery(getErrorMessage(error)),
      });
    }
  }

  if (typeof webNavigator.mediaDevices?.getUserMedia !== "function") {
    return status({
      state: "unavailable",
      detail: messages.microphone.captureUnavailable,
    });
  }

  return status({
    state: "unknown",
    detail: messages.microphone.permissionStatusUnavailable,
  });
}

async function requestNotificationPermissionStatus(
  messages: DesktopPermissionMessages,
): Promise<DesktopPermissionStatus> {
  if (isNative) {
    return status({
      state: "unavailable",
      detail: messages.notifications.requestWebOnly,
    });
  }

  const NotificationConstructor = getWebNotificationConstructor();
  if (NotificationConstructor && typeof NotificationConstructor.requestPermission === "function") {
    try {
      const permission = await NotificationConstructor.requestPermission();
      return mapNotificationPermissionString(permission, messages);
    } catch (error) {
      return status({
        state: "unknown",
        detail: messages.notifications.failedRequest(getErrorMessage(error)),
      });
    }
  }

  return status({
    state: "unavailable",
    detail: messages.notifications.requestApiUnavailable,
  });
}

async function requestMicrophonePermissionStatus(
  messages: DesktopPermissionMessages,
): Promise<DesktopPermissionStatus> {
  if (isNative) {
    return status({
      state: "unavailable",
      detail: messages.microphone.requestWebOnly,
    });
  }

  const webNavigator = getNavigatorLike();
  if (!webNavigator || typeof webNavigator.mediaDevices?.getUserMedia !== "function") {
    return status({
      state: "unavailable",
      detail: messages.microphone.requestCaptureUnavailable,
    });
  }

  try {
    const stream = await webNavigator.mediaDevices.getUserMedia({ audio: true });
    const tracks = stream && typeof stream.getTracks === "function" ? stream.getTracks() : [];
    tracks.forEach((track) => {
      if (typeof track.stop === "function") {
        track.stop();
      }
    });
    return await getMicrophonePermissionStatus(messages);
  } catch (error) {
    const errorName = getErrorName(error);
    if (errorName === "NotAllowedError" || errorName === "PermissionDeniedError") {
      return status({
        state: "denied",
        detail: messages.microphone.deniedByUserOrSystem,
      });
    }
    if (errorName === "NotFoundError" || errorName === "DevicesNotFoundError") {
      return status({
        state: "unavailable",
        detail: messages.microphone.noDeviceFound,
      });
    }
    return status({
      state: "unknown",
      detail: messages.microphone.failedRequest(getErrorMessage(error)),
    });
  }
}

export async function requestDesktopPermission(input: {
  kind: DesktopPermissionKind;
  messages?: DesktopPermissionMessages;
}): Promise<DesktopPermissionStatus> {
  const messages = input.messages ?? DEFAULT_DESKTOP_PERMISSION_MESSAGES;
  if (input.kind === "notifications") {
    return await requestNotificationPermissionStatus(messages);
  }
  return await requestMicrophonePermissionStatus(messages);
}

export async function getDesktopPermissionSnapshot(
  messages: DesktopPermissionMessages = DEFAULT_DESKTOP_PERMISSION_MESSAGES,
): Promise<DesktopPermissionSnapshot> {
  const [notifications, microphone] = await Promise.all([
    getNotificationPermissionStatus(messages),
    getMicrophonePermissionStatus(messages),
  ]);

  return {
    checkedAt: Date.now(),
    notifications,
    microphone,
  };
}
