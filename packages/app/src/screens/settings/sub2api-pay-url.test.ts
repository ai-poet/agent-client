import { describe, expect, it } from "vitest";
import {
  buildPayCenterApiUrl,
  buildPayCenterOrderStatusUrl,
  buildPayCenterStripePopupUrl,
  buildPayCenterUrl,
} from "./sub2api-pay-url";

describe("sub2api-pay-url", () => {
  it("builds the pay center URL under /pay", () => {
    expect(buildPayCenterUrl("https://ai-coding.cyberspirit.io", "token-123")).toBe(
      "https://ai-coding.cyberspirit.io/pay?token=token-123&theme=dark&ui_mode=embedded&lang=zh",
    );
  });

  it("normalizes explicit pay center languages", () => {
    expect(
      buildPayCenterUrl("https://ai-coding.cyberspirit.io", "token-123", {
        lang: "en-US",
      }),
    ).toBe(
      "https://ai-coding.cyberspirit.io/pay?token=token-123&theme=dark&ui_mode=embedded&lang=en",
    );
  });

  it("routes payment API calls through /pay/api", () => {
    expect(
      buildPayCenterApiUrl(
        "https://ai-coding.cyberspirit.io",
        "/api/orders/my?token=abc&page=1&page_size=20",
      ),
    ).toBe("https://ai-coding.cyberspirit.io/pay/api/orders/my?token=abc&page=1&page_size=20");
  });

  it("can append normalized language to payment API calls", () => {
    expect(
      buildPayCenterApiUrl("https://ai-coding.cyberspirit.io", "/api/orders/my?token=abc", {
        lang: "en-US",
      }),
    ).toBe("https://ai-coding.cyberspirit.io/pay/api/orders/my?token=abc&lang=en");
  });

  it("builds single-order status URLs with access tokens", () => {
    expect(
      buildPayCenterOrderStatusUrl("https://ai-coding.cyberspirit.io", "order-123", "status-456"),
    ).toBe("https://ai-coding.cyberspirit.io/pay/api/orders/order-123?access_token=status-456");
  });

  it("can include normalized language in single-order status URLs", () => {
    expect(
      buildPayCenterOrderStatusUrl("https://ai-coding.cyberspirit.io", "order-123", "status-456", {
        lang: "en-US",
      }),
    ).toBe(
      "https://ai-coding.cyberspirit.io/pay/api/orders/order-123?access_token=status-456&lang=en",
    );
  });

  it("builds stripe popup URLs under /pay/stripe-popup", () => {
    expect(
      buildPayCenterStripePopupUrl({
        endpoint: "https://ai-coding.cyberspirit.io",
        orderId: "order-123",
        amount: 12.5,
        accessToken: "status-456",
        method: "card",
        lang: "en-US",
      }),
    ).toBe(
      "https://ai-coding.cyberspirit.io/pay/stripe-popup?order_id=order-123&amount=12.5&theme=dark&access_token=status-456&method=card&lang=en",
    );
  });
});
