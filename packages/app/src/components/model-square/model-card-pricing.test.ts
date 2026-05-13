import { describe, expect, it } from "vitest";
import {
  formatActualPaidPrice,
  formatUsdPrice,
  getActualPaidSectionLabel,
} from "./model-card-pricing";

describe("model card pricing", () => {
  it("falls back to balance USD when actual paid config is unavailable", () => {
    expect(formatActualPaidPrice(0.25, null)).toBe("$0.2500");
    expect(getActualPaidSectionLabel(null)).toBe("Balance Price");
  });

  it("converts balance price to CNY actual paid", () => {
    expect(
      formatActualPaidPrice(0.25, {
        balanceCreditCnyPerUsd: 7.2,
        usdExchangeRate: 6.9,
        locale: "zh-CN",
      }),
    ).toBe("¥1.8000");
    expect(
      getActualPaidSectionLabel({
        balanceCreditCnyPerUsd: 7.2,
        usdExchangeRate: 6.9,
      }),
    ).toBe("Actual Paid");
  });

  it("matches the sub2api English display by converting actual CNY back to USD cash", () => {
    expect(
      formatActualPaidPrice(0.25, {
        balanceCreditCnyPerUsd: 7.2,
        usdExchangeRate: 6.9,
        locale: "en-US",
      }),
    ).toBe("$0.2609");
  });

  it("falls back to balance USD in English when USD exchange rate is unavailable", () => {
    expect(
      formatActualPaidPrice(0.25, {
        balanceCreditCnyPerUsd: 7.2,
        usdExchangeRate: null,
        locale: "en-US",
      }),
    ).toBe("$0.2500");
  });

  it("localizes actual paid section labels", () => {
    expect(
      getActualPaidSectionLabel(
        {
          balanceCreditCnyPerUsd: 7.2,
          usdExchangeRate: 6.9,
        },
        "zh-CN",
      ),
    ).toBe("实付价格");
    expect(getActualPaidSectionLabel(null, "zh-CN")).toBe("余额价格");
  });

  it("formats official USD prices with stable precision", () => {
    expect(formatUsdPrice(125)).toBe("$125.00");
    expect(formatUsdPrice(12.5)).toBe("$12.500");
    expect(formatUsdPrice(1.25)).toBe("$1.2500");
  });
});
