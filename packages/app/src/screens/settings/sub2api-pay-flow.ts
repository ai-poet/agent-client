export type Sub2APIPaymentOrderFlow = "redirect" | "qr" | "stripe";

export type Sub2APIPaymentOrderFlowInput = {
  paymentType: string;
  qrCode?: string | null;
  clientSecret?: string | null;
};

const REDIRECT_PAYMENT_PREFIXES = ["alipay", "wxpay", "bank"] as const;

function normalizePaymentType(paymentType: string | null | undefined): string {
  return paymentType?.trim().toLowerCase() ?? "";
}

export function isSub2APIRedirectPaymentType(paymentType: string | null | undefined): boolean {
  const type = normalizePaymentType(paymentType);
  return REDIRECT_PAYMENT_PREFIXES.some((prefix) => type.startsWith(prefix));
}

export function resolveSub2APIPaymentOrderFlow(
  order: Sub2APIPaymentOrderFlowInput,
): Sub2APIPaymentOrderFlow {
  if (order.clientSecret) {
    return "stripe";
  }
  if (isSub2APIRedirectPaymentType(order.paymentType)) {
    return "redirect";
  }
  if (order.qrCode) {
    return "qr";
  }
  return "redirect";
}
