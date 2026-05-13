import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ActivityIndicator, Image, Pressable, Text, TextInput, View } from "react-native";
import * as QRCode from "qrcode";
import { StyleSheet } from "react-native-unistyles";
import { AdaptiveModalSheet } from "@/components/adaptive-modal-sheet";
import { Button } from "@/components/ui/button";
import { useToast } from "@/contexts/toast-context";
import { isWeb } from "@/constants/platform";
import {
  buildPayCenterApiUrl,
  buildPayCenterOrderStatusUrl,
  buildPayCenterStripePopupUrl,
  buildPayCenterUrl,
} from "@/screens/settings/sub2api-pay-url";
import { openExternalUrl } from "@/utils/open-external-url";
import {
  filterSub2APIPaymentTypesByLocale,
  getSub2APIMessages,
  getSub2APIPaymentLabel,
  type Sub2APILocale,
} from "@/i18n/sub2api";
import { useSub2APILocale } from "@/hooks/use-sub2api-locale";

export interface Sub2APIPayModalProps {
  visible: boolean;
  endpoint: string;
  accessToken: string | null;
  onClose: () => void;
  onCompleted?: () => void;
}

interface PaymentCenterMethodLimit {
  available: boolean;
  remaining: number | null;
  singleMin?: number;
  singleMax?: number;
  feeRate?: number;
}

interface PaymentCenterConfig {
  userDisplayName: string;
  userBalance: number | null;
  enabledPaymentTypes: string[];
  methodLimits: Record<string, PaymentCenterMethodLimit>;
  minAmount: number;
  maxAmount: number;
  maxPendingOrders: number;
  pendingCount: number;
  usdExchangeRate: number | null;
  balanceCreditCnyPerUsd: number | null;
  stripePublishableKey: string | null;
}

interface PaymentCenterCreateOrderResponse {
  orderId: string;
  amount: number;
  payAmount: number | null;
  status: string;
  paymentType: string;
  payUrl: string | null;
  qrCode: string | null;
  clientSecret: string | null;
  expiresAt: string;
  statusAccessToken: string;
}

interface PaymentCenterOrderStatus {
  id: string;
  status: string;
  expiresAt: string;
  paymentSuccess: boolean;
  rechargeSuccess: boolean;
  rechargeStatus: string;
  failedReason: string | null;
}

type ModalStage = "loading-config" | "form" | "paying" | "result" | "error";

type OrderFlowKind = "redirect" | "qr" | "stripe";

type StageDescriptor = {
  title: string;
  message: string;
  tone: "default" | "success" | "warning" | "error";
};

const DEFAULT_THEME = "dark";
const QUICK_AMOUNTS = [10, 20, 50, 100, 200, 500];
const AMOUNT_PATTERN = /^\d*(\.\d{0,2})?$/;
const TERMINAL_ORDER_STATUSES = new Set(["FAILED", "CANCELLED", "EXPIRED", "COMPLETED"]);

function readJsonSafe<T = unknown>(raw: string): T | null {
  try {
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

function readNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function readString(value: unknown): string | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function getApiErrorMessage(payload: unknown, fallback: string): string {
  if (typeof payload === "object" && payload !== null) {
    const data = payload as Record<string, unknown>;
    const error = readString(data.error);
    if (error) {
      return error;
    }
    const message = readString(data.message);
    if (message) {
      return message;
    }
  }
  return fallback;
}

function formatUsd(value: number | null | undefined): string {
  return typeof value === "number" && Number.isFinite(value) ? `$${value.toFixed(2)}` : "--";
}

function formatCny(value: number | null | undefined): string {
  return typeof value === "number" && Number.isFinite(value) ? `¥${value.toFixed(2)}` : "--";
}

function isStripePaymentType(paymentType: string): boolean {
  return paymentType === "stripe";
}

function resolveOrderFlow(order: PaymentCenterCreateOrderResponse): OrderFlowKind {
  if (order.clientSecret) {
    return "stripe";
  }
  if (order.qrCode) {
    return "qr";
  }
  return "redirect";
}

function formatTimeRemaining(expiresAt: string, expiredLabel: string): string {
  const expiresAtMs = new Date(expiresAt).getTime();
  if (!Number.isFinite(expiresAtMs)) {
    return "--";
  }
  const diff = expiresAtMs - Date.now();
  if (diff <= 0) {
    return expiredLabel;
  }
  const totalSeconds = Math.floor(diff / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

function isTerminalOrderStatus(status: PaymentCenterOrderStatus): boolean {
  return TERMINAL_ORDER_STATUSES.has(status.status.toUpperCase());
}

function createSeedOrderStatus(order: PaymentCenterCreateOrderResponse): PaymentCenterOrderStatus {
  return {
    id: order.orderId,
    status: order.status,
    expiresAt: order.expiresAt,
    paymentSuccess: false,
    rechargeSuccess: false,
    rechargeStatus: "not_paid",
    failedReason: null,
  };
}

function describeOrderStatus(
  status: PaymentCenterOrderStatus,
  locale: Sub2APILocale,
): StageDescriptor {
  const text = getSub2APIMessages(locale).pay.status;
  if (status.rechargeSuccess || status.status.toUpperCase() === "COMPLETED") {
    return {
      title: text.rechargeCompleteTitle,
      message: text.rechargeCompleteMessage,
      tone: "success",
    };
  }
  if (status.paymentSuccess) {
    if (status.rechargeStatus === "paid_pending" || status.rechargeStatus === "recharging") {
      return {
        title: text.paymentReceivedTitle,
        message: text.paymentReceivedMessage,
        tone: "success",
      };
    }
    if (status.rechargeStatus === "failed") {
      return {
        title: text.rechargeUnfinishedTitle,
        message: status.failedReason ?? text.rechargeUnfinishedMessage,
        tone: "warning",
      };
    }
  }

  const normalizedStatus = status.status.toUpperCase();
  if (normalizedStatus === "PENDING") {
    return {
      title: text.awaitingPaymentTitle,
      message: text.awaitingPaymentMessage,
      tone: "default",
    };
  }
  if (normalizedStatus === "FAILED") {
    return {
      title: text.paymentFailedTitle,
      message: status.failedReason ?? text.paymentFailedMessage,
      tone: "error",
    };
  }
  if (normalizedStatus === "CANCELLED") {
    return {
      title: text.orderCancelledTitle,
      message: text.orderCancelledMessage,
      tone: "warning",
    };
  }
  if (normalizedStatus === "EXPIRED") {
    return {
      title: text.orderExpiredTitle,
      message: text.orderExpiredMessage,
      tone: "warning",
    };
  }

  return {
    title: text.statusUpdatedTitle,
    message: text.statusUpdatedMessage,
    tone: "default",
  };
}

function canUseEmbeddedStripePopup(endpoint: string): boolean {
  if (!isWeb || typeof window === "undefined") {
    return false;
  }
  try {
    return new URL(endpoint).origin === window.location.origin;
  } catch {
    return false;
  }
}

function estimateSettlementSummary(input: {
  amount: number | null;
  feeRate: number;
  balanceCreditCnyPerUsd: number | null;
  text: ReturnType<typeof getSub2APIMessages>["pay"];
}): { creditedUsd: string; estimatedPay: string; feeText: string | null; rateText: string | null } {
  const amount = input.amount;
  const creditedUsd = formatUsd(amount);
  const rateText =
    typeof input.balanceCreditCnyPerUsd === "number" &&
    Number.isFinite(input.balanceCreditCnyPerUsd)
      ? input.text.estimatedSettlement(
          formatCny(amount == null ? null : amount * input.balanceCreditCnyPerUsd),
        )
      : null;

  if (amount == null) {
    return {
      creditedUsd,
      estimatedPay: "--",
      feeText: input.feeRate > 0 ? input.text.methodFee(input.feeRate) : null,
      rateText,
    };
  }

  const settlementBase =
    typeof input.balanceCreditCnyPerUsd === "number" &&
    Number.isFinite(input.balanceCreditCnyPerUsd)
      ? amount * input.balanceCreditCnyPerUsd
      : amount;
  const estimatedPay = settlementBase * (1 + input.feeRate / 100);

  return {
    creditedUsd,
    estimatedPay:
      typeof input.balanceCreditCnyPerUsd === "number" &&
      Number.isFinite(input.balanceCreditCnyPerUsd)
        ? formatCny(estimatedPay)
        : formatUsd(estimatedPay),
    feeText: input.feeRate > 0 ? input.text.methodFee(input.feeRate) : null,
    rateText,
  };
}

export function Sub2APIPayModal({
  visible,
  endpoint,
  accessToken,
  onClose,
  onCompleted,
}: Sub2APIPayModalProps) {
  const toast = useToast();
  const [stage, setStage] = useState<ModalStage>("loading-config");
  const [config, setConfig] = useState<PaymentCenterConfig | null>(null);
  const [selectedPaymentType, setSelectedPaymentType] = useState<string | null>(null);
  const [amountText, setAmountText] = useState("10");
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [activeOrder, setActiveOrder] = useState<PaymentCenterCreateOrderResponse | null>(null);
  const [activeOrderStatus, setActiveOrderStatus] = useState<PaymentCenterOrderStatus | null>(null);
  const [qrCodeDataUrl, setQrCodeDataUrl] = useState<string | null>(null);
  const [isLoadingConfig, setIsLoadingConfig] = useState(false);
  const [isCreatingOrder, setIsCreatingOrder] = useState(false);
  const [isPollingOrder, setIsPollingOrder] = useState(false);
  const [isCancellingOrder, setIsCancellingOrder] = useState(false);
  const [isOpeningStripePopup, setIsOpeningStripePopup] = useState(false);
  const [stripePopupBlocked, setStripePopupBlocked] = useState(false);
  const [countdownTick, setCountdownTick] = useState(0);
  const pollingStartedAtRef = useRef<number | null>(null);
  const paymentCompletionHandledRef = useRef(false);
  const clientLocale = useSub2APILocale();
  const text = useMemo(() => getSub2APIMessages(clientLocale).pay, [clientLocale]);

  const payCenterUrl = useMemo(() => {
    if (!accessToken) {
      return null;
    }
    try {
      return buildPayCenterUrl(endpoint, accessToken, {
        lang: clientLocale,
        theme: DEFAULT_THEME,
        uiMode: "embedded",
      });
    } catch {
      return null;
    }
  }, [accessToken, clientLocale, endpoint]);

  const selectedMethodLimit = useMemo(() => {
    if (!config || !selectedPaymentType) {
      return null;
    }
    return config.methodLimits[selectedPaymentType] ?? null;
  }, [config, selectedPaymentType]);

  const parsedAmount = useMemo(() => {
    if (!amountText.trim()) {
      return null;
    }
    const next = Number(amountText);
    return Number.isFinite(next) && next > 0 ? next : null;
  }, [amountText]);

  const effectiveMinAmount = useMemo(() => {
    if (!config) {
      return 1;
    }
    const methodMin = readNumber(selectedMethodLimit?.singleMin);
    return methodMin && methodMin > 0 ? Math.max(config.minAmount, methodMin) : config.minAmount;
  }, [config, selectedMethodLimit]);

  const effectiveMaxAmount = useMemo(() => {
    if (!config) {
      return 1000;
    }
    const methodMax = readNumber(selectedMethodLimit?.singleMax);
    return methodMax && methodMax > 0 ? methodMax : config.maxAmount;
  }, [config, selectedMethodLimit]);

  const feeRate = useMemo(() => {
    const value = readNumber(selectedMethodLimit?.feeRate);
    return value && value > 0 ? value : 0;
  }, [selectedMethodLimit]);

  const paymentSummary = useMemo(
    () =>
      estimateSettlementSummary({
        amount: parsedAmount,
        feeRate,
        balanceCreditCnyPerUsd: config?.balanceCreditCnyPerUsd ?? null,
        text,
      }),
    [config?.balanceCreditCnyPerUsd, feeRate, parsedAmount, text],
  );

  const canUseStripePopup = useMemo(() => canUseEmbeddedStripePopup(endpoint), [endpoint]);

  const activeFlow = useMemo(
    () => (activeOrder ? resolveOrderFlow(activeOrder) : null),
    [activeOrder],
  );

  const currentOrderStatus =
    activeOrderStatus ?? (activeOrder ? createSeedOrderStatus(activeOrder) : null);
  const statusDescriptor = currentOrderStatus
    ? describeOrderStatus(currentOrderStatus, clientLocale)
    : null;
  const timeRemaining = useMemo(() => {
    void countdownTick;
    return currentOrderStatus
      ? formatTimeRemaining(currentOrderStatus.expiresAt, text.expired)
      : "--";
  }, [countdownTick, currentOrderStatus, text.expired]);

  const resetOrderState = useCallback(() => {
    setActiveOrder(null);
    setActiveOrderStatus(null);
    setQrCodeDataUrl(null);
    setIsPollingOrder(false);
    setIsCancellingOrder(false);
    setIsOpeningStripePopup(false);
    setStripePopupBlocked(false);
    pollingStartedAtRef.current = null;
    paymentCompletionHandledRef.current = false;
  }, []);

  const resetAllState = useCallback(() => {
    setStage("loading-config");
    setConfig(null);
    setSelectedPaymentType(null);
    setAmountText("10");
    setErrorMessage(null);
    setIsLoadingConfig(false);
    setIsCreatingOrder(false);
    resetOrderState();
  }, [resetOrderState]);

  const loadPayConfig = useCallback(async () => {
    if (!visible || !accessToken) {
      return;
    }

    setStage("loading-config");
    setIsLoadingConfig(true);
    setErrorMessage(null);
    resetOrderState();

    try {
      const ordersUrl = buildPayCenterApiUrl(
        endpoint,
        `/api/orders/my?token=${encodeURIComponent(accessToken)}&page=1&page_size=20`,
        { lang: clientLocale },
      );
      const ordersResponse = await fetch(ordersUrl, {
        headers: { "Accept-Language": clientLocale },
      });
      const ordersPayload = readJsonSafe(await ordersResponse.text());
      if (!ordersResponse.ok) {
        throw new Error(getApiErrorMessage(ordersPayload, text.failedLoadOrders));
      }

      const userId = readNumber((ordersPayload as { user?: { id?: unknown } } | null)?.user?.id);
      if (!userId || userId <= 0) {
        throw new Error(text.paymentUserUnavailable);
      }

      const userDisplayName =
        readString(
          (
            ordersPayload as {
              user?: { displayName?: unknown; username?: unknown; email?: unknown };
            } | null
          )?.user?.displayName,
        ) ??
        readString(
          (ordersPayload as { user?: { username?: unknown; email?: unknown } } | null)?.user
            ?.username,
        ) ??
        readString((ordersPayload as { user?: { email?: unknown } } | null)?.user?.email) ??
        text.accountFallback;
      const userBalance = readNumber(
        (ordersPayload as { user?: { balance?: unknown } } | null)?.user?.balance,
      );
      const pendingCount =
        readNumber(
          (ordersPayload as { summary?: { pending?: unknown } } | null)?.summary?.pending,
        ) ?? 0;

      const userUrl = buildPayCenterApiUrl(
        endpoint,
        `/api/user?user_id=${userId}&token=${encodeURIComponent(accessToken)}`,
        { lang: clientLocale },
      );
      const userResponse = await fetch(userUrl, {
        method: "GET",
        headers: { "Accept-Language": clientLocale },
      });
      const userPayload = readJsonSafe(await userResponse.text());
      if (!userResponse.ok) {
        throw new Error(getApiErrorMessage(userPayload, text.failedLoadConfig));
      }

      const configPayload =
        (userPayload as { config?: Record<string, unknown> } | null)?.config ?? {};
      const rawEnabledPaymentTypes = Array.isArray(configPayload.enabledPaymentTypes)
        ? configPayload.enabledPaymentTypes.filter(
            (entry): entry is string => typeof entry === "string",
          )
        : [];
      const enabledPaymentTypes = filterSub2APIPaymentTypesByLocale(
        rawEnabledPaymentTypes,
        clientLocale,
      );
      const methodLimits =
        typeof configPayload.methodLimits === "object" && configPayload.methodLimits !== null
          ? (configPayload.methodLimits as Record<string, PaymentCenterMethodLimit>)
          : {};

      const nextConfig: PaymentCenterConfig = {
        userDisplayName,
        userBalance,
        enabledPaymentTypes,
        methodLimits,
        minAmount: readNumber(configPayload.minAmount) ?? 1,
        maxAmount: readNumber(configPayload.maxAmount) ?? 1000,
        maxPendingOrders: readNumber(configPayload.maxPendingOrders) ?? 3,
        pendingCount,
        usdExchangeRate: readNumber(configPayload.usdExchangeRate),
        balanceCreditCnyPerUsd: readNumber(configPayload.balanceCreditCnyPerUsd),
        stripePublishableKey: readString(configPayload.stripePublishableKey),
      };

      setConfig(nextConfig);
      setSelectedPaymentType((current) => {
        if (current && nextConfig.enabledPaymentTypes.includes(current)) {
          return current;
        }
        return nextConfig.enabledPaymentTypes[0] ?? null;
      });
      setAmountText(String(Math.max(1, nextConfig.minAmount)));
      setStage("form");
    } catch (error) {
      setConfig(null);
      setStage("error");
      setErrorMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setIsLoadingConfig(false);
    }
  }, [accessToken, clientLocale, endpoint, resetOrderState, text, visible]);

  useEffect(() => {
    if (!visible) {
      resetAllState();
      return;
    }
    void loadPayConfig();
  }, [loadPayConfig, resetAllState, visible]);

  const handleAmountChange = useCallback((value: string) => {
    if (!AMOUNT_PATTERN.test(value)) {
      return;
    }
    setAmountText(value);
  }, []);

  const openPaymentTarget = useCallback(async (url: string | null) => {
    if (!url) {
      return;
    }
    await openExternalUrl(url);
  }, []);

  const openStripePopup = useCallback(
    (order: PaymentCenterCreateOrderResponse, paymentMethod = "card"): boolean => {
      if (!config?.stripePublishableKey || !order.clientSecret || !canUseStripePopup || !isWeb) {
        return false;
      }
      const popupUrl = buildPayCenterStripePopupUrl({
        endpoint,
        orderId: order.orderId,
        amount: order.payAmount ?? order.amount,
        accessToken: order.statusAccessToken,
        method: paymentMethod,
        lang: clientLocale,
        theme: DEFAULT_THEME,
      });

      const popup = window.open(popupUrl, "stripe_payment", "width=500,height=700,scrollbars=yes");
      if (!popup || popup.closed) {
        setStripePopupBlocked(true);
        return false;
      }

      const targetOrigin = new URL(popupUrl).origin;
      const onReady = (event: MessageEvent) => {
        if (
          event.source !== popup ||
          event.origin !== targetOrigin ||
          event.data?.type !== "STRIPE_POPUP_READY"
        ) {
          return;
        }
        window.removeEventListener("message", onReady);
        popup.postMessage(
          {
            type: "STRIPE_POPUP_INIT",
            clientSecret: order.clientSecret,
            publishableKey: config.stripePublishableKey,
          },
          targetOrigin,
        );
      };

      window.addEventListener("message", onReady);
      setStripePopupBlocked(false);
      return true;
    },
    [canUseStripePopup, clientLocale, config?.stripePublishableKey, endpoint],
  );

  const handleCreateOrder = useCallback(async () => {
    if (!accessToken || !config) {
      setErrorMessage(text.signInAgain);
      return;
    }
    if (!selectedPaymentType) {
      setErrorMessage(text.selectPaymentMethod);
      return;
    }

    if (isStripePaymentType(selectedPaymentType) && !canUseStripePopup) {
      setErrorMessage(text.stripeFullCenterRequired);
      if (payCenterUrl) {
        await openPaymentTarget(payCenterUrl);
      }
      return;
    }

    if (config.pendingCount >= config.maxPendingOrders) {
      setErrorMessage(text.pendingOrders(config.pendingCount));
      return;
    }
    if (selectedMethodLimit?.available === false) {
      setErrorMessage(text.methodUnavailable);
      return;
    }
    if (parsedAmount == null) {
      setErrorMessage(text.enterValidAmount);
      return;
    }
    if (parsedAmount < effectiveMinAmount || parsedAmount > effectiveMaxAmount) {
      setErrorMessage(text.rechargeAmountRange(effectiveMinAmount, effectiveMaxAmount));
      return;
    }

    setIsCreatingOrder(true);
    setErrorMessage(null);
    try {
      const ordersUrl = buildPayCenterApiUrl(endpoint, "/api/orders", { lang: clientLocale });
      const response = await fetch(ordersUrl, {
        method: "POST",
        headers: { "Accept-Language": clientLocale, "Content-Type": "application/json" },
        body: JSON.stringify({
          token: accessToken,
          amount: parsedAmount,
          payment_type: selectedPaymentType,
          is_mobile: false,
        }),
      });
      const payload = readJsonSafe(await response.text());
      if (!response.ok) {
        throw new Error(getApiErrorMessage(payload, text.failedCreateOrder));
      }

      const createdOrder: PaymentCenterCreateOrderResponse = {
        orderId:
          readString((payload as { orderId?: unknown } | null)?.orderId) ??
          (() => {
            throw new Error(text.missingOrderId);
          })(),
        amount: readNumber((payload as { amount?: unknown } | null)?.amount) ?? parsedAmount,
        payAmount: readNumber((payload as { payAmount?: unknown } | null)?.payAmount),
        status: readString((payload as { status?: unknown } | null)?.status) ?? "PENDING",
        paymentType:
          readString((payload as { paymentType?: unknown } | null)?.paymentType) ??
          selectedPaymentType,
        payUrl: readString((payload as { payUrl?: unknown } | null)?.payUrl),
        qrCode: readString((payload as { qrCode?: unknown } | null)?.qrCode),
        clientSecret: readString((payload as { clientSecret?: unknown } | null)?.clientSecret),
        expiresAt:
          readString((payload as { expiresAt?: unknown } | null)?.expiresAt) ??
          new Date(Date.now() + 15 * 60 * 1000).toISOString(),
        statusAccessToken:
          readString((payload as { statusAccessToken?: unknown } | null)?.statusAccessToken) ??
          (() => {
            throw new Error(text.missingStatusAccessToken);
          })(),
      };

      setActiveOrder(createdOrder);
      setActiveOrderStatus(createSeedOrderStatus(createdOrder));
      setStage("paying");
      setIsPollingOrder(true);
      pollingStartedAtRef.current = Date.now();
      paymentCompletionHandledRef.current = false;

      const flow = resolveOrderFlow(createdOrder);
      if (flow === "redirect") {
        await openPaymentTarget(createdOrder.payUrl);
      } else if (flow === "stripe") {
        const opened = openStripePopup(createdOrder);
        if (!opened) {
          setErrorMessage(text.stripeWindowBlocked);
        }
      }
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : String(error));
      setStage("form");
    } finally {
      setIsCreatingOrder(false);
    }
  }, [
    accessToken,
    canUseStripePopup,
    clientLocale,
    config,
    effectiveMaxAmount,
    effectiveMinAmount,
    endpoint,
    openPaymentTarget,
    openStripePopup,
    parsedAmount,
    payCenterUrl,
    selectedMethodLimit?.available,
    selectedPaymentType,
    text,
  ]);

  const pollOrderStatus = useCallback(async (): Promise<PaymentCenterOrderStatus | null> => {
    if (!activeOrder) {
      return null;
    }
    const statusUrl = buildPayCenterOrderStatusUrl(
      endpoint,
      activeOrder.orderId,
      activeOrder.statusAccessToken,
      { lang: clientLocale },
    );
    const response = await fetch(statusUrl, {
      headers: { "Accept-Language": clientLocale },
    });
    const payload = readJsonSafe(await response.text());
    if (!response.ok) {
      throw new Error(getApiErrorMessage(payload, text.failedRefreshStatus));
    }

    return {
      id: readString((payload as { id?: unknown } | null)?.id) ?? activeOrder.orderId,
      status: readString((payload as { status?: unknown } | null)?.status) ?? "PENDING",
      expiresAt:
        readString((payload as { expiresAt?: unknown } | null)?.expiresAt) ?? activeOrder.expiresAt,
      paymentSuccess: (payload as { paymentSuccess?: unknown } | null)?.paymentSuccess === true,
      rechargeSuccess: (payload as { rechargeSuccess?: unknown } | null)?.rechargeSuccess === true,
      rechargeStatus:
        readString((payload as { rechargeStatus?: unknown } | null)?.rechargeStatus) ?? "not_paid",
      failedReason: readString((payload as { failedReason?: unknown } | null)?.failedReason),
    };
  }, [activeOrder, clientLocale, endpoint, text.failedRefreshStatus]);

  const refreshBalanceAndClose = useCallback(() => {
    if (paymentCompletionHandledRef.current) {
      return;
    }
    paymentCompletionHandledRef.current = true;
    toast.show(text.paymentSuccessfulToast, { variant: "success" });
    onCompleted?.();
    onClose();
  }, [onClose, onCompleted, text.paymentSuccessfulToast, toast]);

  useEffect(() => {
    if (!currentOrderStatus) {
      return;
    }
    if (
      currentOrderStatus.rechargeSuccess ||
      currentOrderStatus.status.toUpperCase() === "COMPLETED"
    ) {
      refreshBalanceAndClose();
      return;
    }
    if (
      currentOrderStatus.rechargeStatus === "failed" ||
      ["FAILED", "CANCELLED", "EXPIRED"].includes(currentOrderStatus.status.toUpperCase())
    ) {
      setStage("result");
      setIsPollingOrder(false);
    }
  }, [currentOrderStatus, refreshBalanceAndClose]);

  useEffect(() => {
    if (stage !== "paying" || !activeOrder || !isPollingOrder) {
      return;
    }

    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;

    const run = async () => {
      if (cancelled) {
        return;
      }
      try {
        const nextStatus = await pollOrderStatus();
        if (cancelled || !nextStatus) {
          return;
        }
        setActiveOrderStatus(nextStatus);
      } catch (error) {
        if (!cancelled) {
          setErrorMessage(error instanceof Error ? error.message : String(error));
        }
      } finally {
        if (cancelled) {
          return;
        }
        const startedAt = pollingStartedAtRef.current;
        const elapsed = startedAt === null ? 0 : Date.now() - startedAt;
        if (elapsed > 5 * 60 * 1000) {
          setIsPollingOrder(false);
          setStage("result");
          setErrorMessage(text.paymentPollingTimedOut);
          return;
        }
        timer = setTimeout(() => void run(), 2000);
      }
    };

    void run();
    return () => {
      cancelled = true;
      if (timer) {
        clearTimeout(timer);
      }
    };
  }, [activeOrder, isPollingOrder, pollOrderStatus, stage, text.paymentPollingTimedOut]);

  useEffect(() => {
    if (stage !== "paying" || activeFlow !== "qr" || !activeOrder?.qrCode) {
      setQrCodeDataUrl(null);
      return;
    }

    let cancelled = false;
    QRCode.toDataURL(activeOrder.qrCode, {
      errorCorrectionLevel: "M",
      margin: 1,
      width: 560,
    })
      .then((uri) => {
        if (!cancelled) {
          setQrCodeDataUrl(uri);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setQrCodeDataUrl(null);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [activeFlow, activeOrder?.qrCode, stage]);

  useEffect(() => {
    if (stage !== "paying" || !currentOrderStatus) {
      return;
    }
    const timer = setInterval(() => {
      setCountdownTick((value) => value + 1);
    }, 1000);
    return () => clearInterval(timer);
  }, [currentOrderStatus, stage]);

  const handleCancelOrder = useCallback(async () => {
    if (!activeOrder || !accessToken) {
      return;
    }
    setIsCancellingOrder(true);
    setErrorMessage(null);
    try {
      const latest = await pollOrderStatus();
      if (
        latest &&
        (latest.paymentSuccess || latest.rechargeSuccess || isTerminalOrderStatus(latest))
      ) {
        setActiveOrderStatus(latest);
        return;
      }

      const cancelUrl = buildPayCenterApiUrl(
        endpoint,
        `/api/orders/${activeOrder.orderId}/cancel`,
        {
          lang: clientLocale,
        },
      );
      const cancelResponse = await fetch(cancelUrl, {
        method: "POST",
        headers: { "Accept-Language": clientLocale, "Content-Type": "application/json" },
        body: JSON.stringify({ token: accessToken }),
      });

      if (!cancelResponse.ok) {
        const cancelPayload = readJsonSafe(await cancelResponse.text());
        const refreshed = await pollOrderStatus().catch(() => null);
        if (refreshed) {
          setActiveOrderStatus(refreshed);
          return;
        }
        throw new Error(getApiErrorMessage(cancelPayload, text.failedCancelOrder));
      }

      const cancelledStatus: PaymentCenterOrderStatus = {
        id: activeOrder.orderId,
        status: "CANCELLED",
        expiresAt: activeOrder.expiresAt,
        paymentSuccess: false,
        rechargeSuccess: false,
        rechargeStatus: "closed",
        failedReason: null,
      };
      setActiveOrderStatus(cancelledStatus);
      setStage("result");
      setIsPollingOrder(false);
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setIsCancellingOrder(false);
    }
  }, [accessToken, activeOrder, clientLocale, endpoint, pollOrderStatus, text.failedCancelOrder]);

  const handleBackToForm = useCallback(() => {
    setErrorMessage(null);
    setStage("form");
    void loadPayConfig();
  }, [loadPayConfig]);

  const mainCtaLabel = useMemo(() => {
    if (isLoadingConfig) {
      return text.loadingCta;
    }
    if (isCreatingOrder) {
      return text.creatingOrder;
    }
    if (selectedPaymentType && isStripePaymentType(selectedPaymentType) && !canUseStripePopup) {
      return text.openFullPayCenter;
    }
    return text.continueToPayment;
  }, [canUseStripePopup, isCreatingOrder, isLoadingConfig, selectedPaymentType, text]);

  const mainCtaDisabled =
    isLoadingConfig ||
    isCreatingOrder ||
    !config ||
    !selectedPaymentType ||
    config.enabledPaymentTypes.length === 0 ||
    selectedMethodLimit?.available === false;

  const quickAmounts = useMemo(
    () =>
      QUICK_AMOUNTS.filter((value) => value >= effectiveMinAmount && value <= effectiveMaxAmount),
    [effectiveMaxAmount, effectiveMinAmount],
  );

  return (
    <AdaptiveModalSheet
      visible={visible}
      onClose={onClose}
      title={text.addBalance}
      scrollable
      snapPoints={["80%", "95%"]}
    >
      <View style={styles.root}>
        {stage === "loading-config" ? (
          <View style={styles.loadingCard}>
            <ActivityIndicator />
            <Text style={styles.loadingText}>{text.loadingRechargeOptions}</Text>
          </View>
        ) : null}

        {stage === "error" ? (
          <View style={styles.errorCard}>
            <Text style={styles.errorTitle}>{text.unableToLoadRechargeOptions}</Text>
            <Text style={styles.errorText}>{errorMessage ?? text.fullPayCenterFallback}</Text>
            {payCenterUrl ? (
              <Button
                size="sm"
                onPress={() => void openPaymentTarget(payCenterUrl)}
                style={styles.primaryCta}
              >
                {text.openFullPayCenter}
              </Button>
            ) : null}
          </View>
        ) : null}

        {stage === "form" && config ? (
          <View style={styles.formStack}>
            <View style={styles.accountCard}>
              <Text style={styles.accountEyebrow}>{text.rechargeAccount}</Text>
              <Text style={styles.accountName}>{config.userDisplayName}</Text>
              <Text style={styles.accountHint}>
                {text.currentBalance}:{" "}
                <Text style={styles.accountBalance}>{formatUsd(config.userBalance)}</Text>
              </Text>
            </View>

            <View style={styles.sectionCard}>
              <Text style={styles.fieldLabel}>{text.rechargeAmount}</Text>
              <View style={styles.quickAmountGrid}>
                {quickAmounts.map((value) => {
                  const selected = amountText.trim() === String(value);
                  return (
                    <Pressable
                      key={value}
                      onPress={() => setAmountText(String(value))}
                      style={({ pressed }) => [
                        styles.quickAmountChip,
                        selected && styles.quickAmountChipSelected,
                        pressed && styles.buttonPressed,
                      ]}
                    >
                      <Text
                        style={[
                          styles.quickAmountChipText,
                          selected && styles.quickAmountChipTextSelected,
                        ]}
                      >
                        ${value}
                      </Text>
                    </Pressable>
                  );
                })}
              </View>

              <TextInput
                value={amountText}
                onChangeText={handleAmountChange}
                keyboardType="decimal-pad"
                autoCapitalize="none"
                autoCorrect={false}
                placeholder={`${effectiveMinAmount}`}
                style={styles.textInput}
              />
              <Text style={styles.rangeHint}>
                {text.creditedRange(
                  paymentSummary.creditedUsd,
                  formatUsd(effectiveMinAmount),
                  formatUsd(effectiveMaxAmount),
                )}
              </Text>
              {paymentSummary.rateText ? (
                <Text style={styles.rangeHint}>{paymentSummary.rateText}</Text>
              ) : null}
              {paymentSummary.feeText ? (
                <Text style={styles.rangeHint}>{paymentSummary.feeText}</Text>
              ) : null}
              {paymentSummary.estimatedPay !== "--" ? (
                <Text style={styles.rangeHint}>
                  {text.estimatedAmountToPay(paymentSummary.estimatedPay)}
                </Text>
              ) : null}
            </View>

            <View style={styles.sectionCard}>
              <Text style={styles.fieldLabel}>{text.paymentMethod}</Text>
              <View style={styles.paymentTypeList}>
                {config.enabledPaymentTypes.map((type) => {
                  const selected = selectedPaymentType === type;
                  return (
                    <Pressable
                      key={type}
                      onPress={() => setSelectedPaymentType(type)}
                      style={({ pressed }) => [
                        styles.paymentTypeChip,
                        selected && styles.paymentTypeChipSelected,
                        pressed && styles.buttonPressed,
                      ]}
                    >
                      <Text
                        style={[
                          styles.paymentTypeChipText,
                          selected && styles.paymentTypeChipTextSelected,
                        ]}
                      >
                        {getSub2APIPaymentLabel(type, clientLocale)}
                      </Text>
                    </Pressable>
                  );
                })}
              </View>
              {selectedMethodLimit?.available === false ? (
                <Text style={styles.errorText}>{text.methodUnavailable}</Text>
              ) : null}
              {selectedMethodLimit?.remaining != null ? (
                <Text style={styles.rangeHint}>
                  {text.remainingDailyQuota(formatUsd(selectedMethodLimit.remaining))}
                </Text>
              ) : null}
              {selectedPaymentType &&
              isStripePaymentType(selectedPaymentType) &&
              !canUseStripePopup ? (
                <Text style={styles.rangeHint}>{text.stripeRuntimeHint}</Text>
              ) : null}
            </View>

            {errorMessage ? <Text style={styles.errorText}>{errorMessage}</Text> : null}

            <Button
              variant="default"
              size="sm"
              onPress={() => void handleCreateOrder()}
              disabled={mainCtaDisabled}
              style={styles.primaryCta}
            >
              {mainCtaLabel}
            </Button>
          </View>
        ) : null}

        {stage === "paying" && activeOrder && currentOrderStatus && statusDescriptor ? (
          <View style={styles.formStack}>
            <View style={styles.statusCard}>
              <Text
                style={[
                  styles.statusTitle,
                  statusDescriptor.tone === "success"
                    ? styles.statusTitleSuccess
                    : statusDescriptor.tone === "warning"
                      ? styles.statusTitleWarning
                      : statusDescriptor.tone === "error"
                        ? styles.statusTitleError
                        : null,
                ]}
              >
                {statusDescriptor.title}
              </Text>
              <Text style={styles.statusHint}>{statusDescriptor.message}</Text>
              <Text style={styles.statusMeta}>
                {text.order} {activeOrder.orderId} · {text.timeRemaining} {timeRemaining}
              </Text>
              <Text style={styles.statusMeta}>
                {text.amountToPay} {formatCny(activeOrder.payAmount ?? activeOrder.amount)} ·{" "}
                {text.credited} {formatUsd(activeOrder.amount)}
              </Text>
            </View>

            {activeFlow === "qr" ? (
              <View style={styles.paymentCard}>
                <Text style={styles.fieldLabel}>
                  {text.qrInstruction(
                    getSub2APIPaymentLabel(activeOrder.paymentType, clientLocale),
                  )}
                </Text>
                <View style={styles.qrContainer}>
                  {qrCodeDataUrl ? (
                    <Image
                      source={{ uri: qrCodeDataUrl }}
                      style={styles.qrImage}
                      resizeMode="contain"
                    />
                  ) : (
                    <ActivityIndicator />
                  )}
                </View>
                {activeOrder.payUrl ? (
                  <Button
                    variant="outline"
                    size="sm"
                    onPress={() => void openPaymentTarget(activeOrder.payUrl)}
                    style={styles.secondaryInlineButton}
                  >
                    {text.openPaymentPage}
                  </Button>
                ) : null}
              </View>
            ) : null}

            {activeFlow === "redirect" ? (
              <View style={styles.paymentCard}>
                <Text style={styles.fieldLabel}>{text.redirectTitle}</Text>
                <Text style={styles.statusHint}>{text.redirectHint}</Text>
                <Button
                  variant="outline"
                  size="sm"
                  onPress={() => void openPaymentTarget(activeOrder.payUrl)}
                  style={styles.secondaryInlineButton}
                >
                  {text.openPaymentPage}
                </Button>
              </View>
            ) : null}

            {activeFlow === "stripe" ? (
              <View style={styles.paymentCard}>
                <Text style={styles.fieldLabel}>{text.stripeCheckout}</Text>
                <Text style={styles.statusHint}>{text.stripeCheckoutHint}</Text>
                {stripePopupBlocked || errorMessage ? (
                  <Text style={styles.errorText}>{errorMessage ?? text.stripeWindowBlocked}</Text>
                ) : null}
                <Button
                  variant="outline"
                  size="sm"
                  onPress={() => {
                    if (!activeOrder) {
                      return;
                    }
                    setIsOpeningStripePopup(true);
                    const opened = openStripePopup(activeOrder);
                    if (!opened) {
                      setErrorMessage(text.stripeCouldNotOpen);
                    }
                    setIsOpeningStripePopup(false);
                  }}
                  style={styles.secondaryInlineButton}
                  disabled={isOpeningStripePopup}
                >
                  {isOpeningStripePopup ? text.opening : text.openSecurePaymentWindow}
                </Button>
              </View>
            ) : null}

            {errorMessage ? <Text style={styles.errorText}>{errorMessage}</Text> : null}

            {!currentOrderStatus.paymentSuccess && !isTerminalOrderStatus(currentOrderStatus) ? (
              <Button
                variant="outline"
                size="sm"
                onPress={() => void handleCancelOrder()}
                disabled={isCancellingOrder}
                style={styles.secondaryInlineButton}
              >
                {isCancellingOrder ? text.cancelling : text.cancelOrder}
              </Button>
            ) : null}
          </View>
        ) : null}

        {stage === "result" && currentOrderStatus && statusDescriptor ? (
          <View style={styles.resultCard}>
            <Text
              style={[
                styles.statusTitle,
                statusDescriptor.tone === "success"
                  ? styles.statusTitleSuccess
                  : statusDescriptor.tone === "warning"
                    ? styles.statusTitleWarning
                    : statusDescriptor.tone === "error"
                      ? styles.statusTitleError
                      : null,
              ]}
            >
              {statusDescriptor.title}
            </Text>
            <Text style={styles.statusHint}>{statusDescriptor.message}</Text>
            <Text style={styles.statusMeta}>
              {text.order} {currentOrderStatus.id}
            </Text>
            {currentOrderStatus.failedReason ? (
              <Text style={styles.errorText}>{currentOrderStatus.failedReason}</Text>
            ) : null}
            <Button
              variant="default"
              size="sm"
              onPress={handleBackToForm}
              style={styles.primaryCta}
            >
              {text.backToRecharge}
            </Button>
          </View>
        ) : null}
      </View>
    </AdaptiveModalSheet>
  );
}

const styles = StyleSheet.create((theme) => ({
  root: {
    minHeight: 320,
    gap: theme.spacing[3],
  },
  formStack: {
    gap: theme.spacing[3],
  },
  loadingCard: {
    minHeight: 180,
    alignItems: "center",
    justifyContent: "center",
    gap: theme.spacing[3],
  },
  loadingText: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  accountCard: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.surface1,
    padding: theme.spacing[4],
    gap: theme.spacing[1],
  },
  accountEyebrow: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    textTransform: "uppercase",
  },
  accountName: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.medium,
  },
  accountHint: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  accountBalance: {
    color: theme.colors.foreground,
    fontWeight: theme.fontWeight.medium,
  },
  sectionCard: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.surface1,
    padding: theme.spacing[4],
    gap: theme.spacing[2],
  },
  paymentCard: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.surface1,
    padding: theme.spacing[4],
    gap: theme.spacing[3],
  },
  errorCard: {
    borderWidth: 1,
    borderColor: theme.colors.destructive,
    borderRadius: theme.borderRadius.lg,
    backgroundColor: "rgba(248,113,113,0.08)",
    padding: theme.spacing[4],
    gap: theme.spacing[2],
  },
  errorTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
  },
  fieldLabel: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
  },
  quickAmountGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[2],
  },
  quickAmountChip: {
    minWidth: 72,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface0,
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[2],
  },
  quickAmountChipSelected: {
    borderColor: theme.colors.accent,
    backgroundColor: theme.colors.accent,
  },
  quickAmountChipText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    fontWeight: theme.fontWeight.medium,
  },
  quickAmountChipTextSelected: {
    color: theme.colors.accentForeground,
  },
  textInput: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface0,
    color: theme.colors.foreground,
    fontSize: theme.fontSize.sm,
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[2],
  },
  rangeHint: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    lineHeight: theme.fontSize.xs * 1.5,
  },
  paymentTypeList: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: theme.spacing[2],
  },
  paymentTypeChip: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.md,
    backgroundColor: theme.colors.surface0,
    paddingHorizontal: theme.spacing[3],
    paddingVertical: theme.spacing[2],
  },
  paymentTypeChipSelected: {
    borderColor: theme.colors.accent,
    backgroundColor: theme.colors.accent,
  },
  paymentTypeChipText: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.xs,
    fontWeight: theme.fontWeight.medium,
  },
  paymentTypeChipTextSelected: {
    color: theme.colors.accentForeground,
  },
  statusCard: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.surface1,
    padding: theme.spacing[4],
    gap: theme.spacing[2],
  },
  resultCard: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.surface1,
    padding: theme.spacing[4],
    gap: theme.spacing[3],
  },
  statusTitle: {
    color: theme.colors.foreground,
    fontSize: theme.fontSize.base,
    fontWeight: theme.fontWeight.medium,
  },
  statusTitleSuccess: {
    color: theme.colors.palette.green[400],
  },
  statusTitleWarning: {
    color: "#d97706",
  },
  statusTitleError: {
    color: theme.colors.destructive,
  },
  statusHint: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
    lineHeight: theme.fontSize.xs * 1.5,
  },
  statusMeta: {
    color: theme.colors.foregroundMuted,
    fontSize: theme.fontSize.xs,
  },
  qrContainer: {
    alignSelf: "center",
    width: 260,
    height: 260,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: theme.borderRadius.lg,
    backgroundColor: theme.colors.surface0,
    padding: theme.spacing[2],
  },
  qrImage: {
    width: "100%",
    height: "100%",
  },
  errorText: {
    color: theme.colors.destructive,
    fontSize: theme.fontSize.xs,
    lineHeight: theme.fontSize.xs * 1.5,
  },
  primaryCta: {
    alignSelf: "stretch",
  },
  secondaryInlineButton: {
    alignSelf: "flex-start",
  },
  buttonPressed: {
    opacity: 0.85,
  },
}));
