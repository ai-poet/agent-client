export function copyArrayBufferViewToBuffer(data: ArrayBufferView): ArrayBuffer {
  const view = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  const out = new Uint8Array(view.byteLength);
  out.set(view);
  return out.buffer;
}

export function normalizeTransportPayload(
  data: string | Uint8Array | ArrayBuffer,
): string | ArrayBuffer {
  if (typeof data === "string" || data instanceof ArrayBuffer) {
    return data;
  }
  return copyArrayBufferViewToBuffer(data);
}

export function extractRelayMessageData(event: unknown): string | ArrayBuffer {
  const raw =
    event && typeof event === "object" && "data" in event
      ? (event as { data: unknown }).data
      : event;
  if (typeof raw === "string") return raw;
  if (raw instanceof ArrayBuffer) return raw;
  if (ArrayBuffer.isView(raw)) {
    return copyArrayBufferViewToBuffer(raw);
  }
  return String(raw ?? "");
}

export type TransportCloseDetails = {
  code?: number;
  reason?: string;
  wasClean?: boolean;
};

function normalizeCloseReason(value: unknown): string | undefined {
  if (typeof value === "string") {
    const trimmed = value.trim();
    return trimmed.length > 0 ? trimmed : undefined;
  }
  if (typeof Buffer !== "undefined" && Buffer.isBuffer(value)) {
    const trimmed = value.toString("utf8").trim();
    return trimmed.length > 0 ? trimmed : undefined;
  }
  if (value instanceof Uint8Array) {
    const decoded =
      typeof TextDecoder !== "undefined"
        ? new TextDecoder().decode(value)
        : typeof Buffer !== "undefined"
          ? Buffer.from(value).toString("utf8")
          : "";
    const trimmed = decoded.trim();
    return trimmed.length > 0 ? trimmed : undefined;
  }
  return undefined;
}

export function getTransportCloseDetails(event?: unknown): TransportCloseDetails {
  if (typeof event === "number" && Number.isFinite(event)) {
    return { code: event };
  }
  if (!event || typeof event !== "object" || event instanceof Error) {
    return {};
  }

  const record = event as { code?: unknown; reason?: unknown; wasClean?: unknown };
  const reason = normalizeCloseReason(record.reason);
  return {
    ...(typeof record.code === "number" && Number.isFinite(record.code)
      ? { code: record.code }
      : {}),
    ...(reason ? { reason } : {}),
    ...(typeof record.wasClean === "boolean" ? { wasClean: record.wasClean } : {}),
  };
}

export function isAbnormalTransportClose(details: TransportCloseDetails): boolean {
  return details.code === 1006;
}

export function getTransportCloseReasonCode(details: TransportCloseDetails): string {
  return isAbnormalTransportClose(details) ? "transport_abnormal_close" : "transport_closed";
}

export function describeTransportClose(event?: unknown): string {
  if (!event) {
    return "Transport closed";
  }
  if (event instanceof Error) {
    return event.message;
  }
  if (typeof event === "string") {
    return event;
  }
  const details = getTransportCloseDetails(event);
  if (details.reason) {
    return details.reason;
  }
  if (typeof event === "object") {
    const record = event as { message?: unknown };
    if (typeof record.message === "string" && record.message.trim().length > 0) {
      return record.message.trim();
    }
  }
  if (typeof details.code === "number") {
    if (isAbnormalTransportClose(details)) {
      return "Connection closed abnormally (code 1006)";
    }
    return `Transport closed (code ${details.code})`;
  }
  return "Transport closed";
}

export function describeTransportError(event?: unknown): string {
  if (!event) {
    return "Transport error";
  }
  if (event instanceof Error) {
    return event.message;
  }
  if (typeof event === "string") {
    return event;
  }
  if (typeof event === "object") {
    const record = event as { message?: unknown };
    if (typeof record.message === "string" && record.message.trim().length > 0) {
      return record.message.trim();
    }
  }
  return "Transport error";
}

export function safeRandomId(): string {
  try {
    if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
      return crypto.randomUUID();
    }
  } catch {
    // ignore
  }
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

export function decodeMessageData(data: unknown): string | null {
  if (data === null || data === undefined) {
    return null;
  }
  if (typeof data === "string") {
    return data;
  }
  if (typeof ArrayBuffer !== "undefined" && data instanceof ArrayBuffer) {
    if (typeof Buffer !== "undefined") {
      return Buffer.from(data).toString("utf8");
    }
    if (typeof TextDecoder !== "undefined") {
      return new TextDecoder().decode(data);
    }
  }
  if (ArrayBuffer.isView(data)) {
    const view = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    if (typeof Buffer !== "undefined") {
      return Buffer.from(view).toString("utf8");
    }
    if (typeof TextDecoder !== "undefined") {
      return new TextDecoder().decode(view);
    }
  }
  if (typeof (data as { toString?: () => string }).toString === "function") {
    return (data as { toString: () => string }).toString();
  }
  return null;
}

export function encodeUtf8String(value: string): Uint8Array {
  if (typeof TextEncoder !== "undefined") {
    return new TextEncoder().encode(value);
  }
  if (typeof Buffer !== "undefined") {
    return new Uint8Array(Buffer.from(value, "utf8"));
  }
  const out = new Uint8Array(value.length);
  for (let i = 0; i < value.length; i++) {
    out[i] = value.charCodeAt(i) & 0xff;
  }
  return out;
}
