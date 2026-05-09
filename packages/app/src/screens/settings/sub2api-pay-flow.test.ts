import { describe, expect, it } from "vitest";
import {
  isSub2APIRedirectPaymentType,
  resolveSub2APIPaymentOrderFlow,
} from "./sub2api-pay-flow";

const baseOrder = {
  orderId: "order-123",
  amount: 10,
  payAmount: 10,
  status: "PENDING",
  payUrl: "https://pay.example.com/order-123",
  expiresAt: "2026-01-01T00:00:00.000Z",
  statusAccessToken: "status-token",
};

describe("sub2api-pay-flow", () => {
  it("treats Alipay, WeChat Pay, and bank payments as browser redirects", () => {
    for (const paymentType of ["alipay", "wxpay_direct", "bank"] as const) {
      expect(isSub2APIRedirectPaymentType(paymentType)).toBe(true);
      expect(
        resolveSub2APIPaymentOrderFlow({
          ...baseOrder,
          paymentType,
          qrCode: "https://qr.example.com",
          clientSecret: null,
        }),
      ).toBe("redirect");
    }
  });

  it("keeps non-redirect QR orders in QR flow", () => {
    expect(
      resolveSub2APIPaymentOrderFlow({
        ...baseOrder,
        paymentType: "custom_qr",
        qrCode: "https://qr.example.com",
        clientSecret: null,
      }),
    ).toBe("qr");
  });

  it("keeps client-secret orders in Stripe flow", () => {
    expect(
      resolveSub2APIPaymentOrderFlow({
        ...baseOrder,
        paymentType: "stripe",
        qrCode: "https://qr.example.com",
        clientSecret: "pi_secret",
      }),
    ).toBe("stripe");
  });
});
