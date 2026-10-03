//! Top-up promotions ("top up $X, get Y"), as the pay service runs them.
//!
//! A port of the pay service's `lib/promotion/calc.ts`. The service decides
//! the bonus inside the order transaction and snapshots it onto the order;
//! the top-up sheet only previews it, so the arithmetic here must match the
//! web page's exactly, in whole cents.

use serde::Deserialize;

/// The largest amount a `Decimal(10,2)` holds; amount plus bonus never
/// exceeds it.
pub const MAX_CREDIT_AMOUNT: f64 = 99_999_999.99;

/// How a promotion's bonus is figured.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BonusType {
    /// A percentage of the amount credited, optionally capped.
    Percent,
    /// A fixed number of dollars. Also what an unknown type reads as, which
    /// is how the service treats anything that is not `percent`.
    #[default]
    #[serde(other)]
    Fixed,
}

/// A running promotion, as `/pay/api/user` lists it under `config.promotions`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Promotion {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// The credited amount (USD) from which the promotion applies.
    #[serde(default)]
    pub min_amount: f64,
    #[serde(default)]
    pub bonus_type: BonusType,
    /// Dollars for a fixed bonus; a percentage (`10` = 10%) otherwise.
    #[serde(default)]
    pub bonus_value: f64,
    /// The cap on a percent bonus, in dollars; `None` is uncapped.
    #[serde(default)]
    pub max_bonus: Option<f64>,
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default)]
    pub starts_at: Option<String>,
    #[serde(default)]
    pub ends_at: Option<String>,
    /// `false` once this user's or the promotion's quota is used up: still
    /// listed, never counted.
    #[serde(default = "default_true")]
    pub available: bool,
}

fn default_true() -> bool {
    true
}

/// The promotion an amount earns, and how much it earns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PromotionMatch<'a> {
    pub rule: &'a Promotion,
    pub bonus: f64,
}

/// "Top up $Z more to reach $T and get $B": the nearest threshold above the
/// current amount that earns a bigger bonus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PromotionHint<'a> {
    pub rule: &'a Promotion,
    pub threshold: f64,
    pub need_more: f64,
    pub bonus: f64,
}

/// The language a promotion is described in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromoLocale {
    Zh,
    En,
    Ja,
}

impl PromoLocale {
    /// From an app locale tag (`zh-CN`, `ja`, `en`, ...).
    pub fn from_locale(tag: &str) -> Self {
        let tag = tag.trim().to_ascii_lowercase();
        if tag.starts_with("zh") {
            Self::Zh
        } else if tag.starts_with("ja") {
            Self::Ja
        } else {
            Self::En
        }
    }
}

/// Dollars to whole cents, rounding as the service does.
pub fn to_cents(value: f64) -> i64 {
    (value * 100.0).round() as i64
}

fn from_cents(cents: i64) -> f64 {
    cents as f64 / 100.0
}

/// The bonus `amount` earns under `rule`, rounded down to the cent; zero
/// below the threshold or for a rule that cannot pay out.
pub fn compute_bonus(amount: f64, rule: &Promotion) -> f64 {
    from_cents(bonus_cents(amount, rule))
}

fn bonus_cents(amount: f64, rule: &Promotion) -> i64 {
    if !amount.is_finite() || amount <= 0.0 {
        return 0;
    }
    if !rule.min_amount.is_finite() || !rule.bonus_value.is_finite() || rule.bonus_value <= 0.0 {
        return 0;
    }
    let amount_cents = to_cents(amount);
    if amount_cents < to_cents(rule.min_amount) {
        return 0;
    }
    let mut cents = match rule.bonus_type {
        BonusType::Percent => {
            let mut cents =
                ((amount_cents as f64 * rule.bonus_value) / 100.0 + 1e-9).floor() as i64;
            if let Some(cap) = rule.max_bonus.filter(|cap| cap.is_finite() && *cap > 0.0) {
                cents = cents.min(to_cents(cap));
            }
            cents
        }
        BonusType::Fixed => to_cents(rule.bonus_value),
    };
    // Amount plus bonus must still fit the service's column.
    let headroom = to_cents(MAX_CREDIT_AMOUNT) - amount_cents;
    cents = cents.min(headroom).max(0);
    cents
}

/// Promotions never stack: the biggest bonus wins, a tie goes to the lower
/// `sort_order`, then the lower id. `None` when nothing pays out.
pub fn pick_best_promotion(amount: f64, rules: &[Promotion]) -> Option<PromotionMatch<'_>> {
    let mut best: Option<(&Promotion, i64)> = None;
    for rule in rules {
        let cents = bonus_cents(amount, rule);
        if cents <= 0 {
            continue;
        }
        let better = match best {
            None => true,
            Some((current, current_cents)) => {
                cents > current_cents
                    || (cents == current_cents
                        && (rule.sort_order < current.sort_order
                            || (rule.sort_order == current.sort_order && rule.id < current.id)))
            }
        };
        if better {
            best = Some((rule, cents));
        }
    }
    best.map(|(rule, cents)| PromotionMatch {
        rule,
        bonus: from_cents(cents),
    })
}

/// The nearest threshold above `amount` that earns more than `amount` does,
/// or `None` when `amount` already earns the most it can get nearby.
pub fn next_promotion_hint(amount: f64, rules: &[Promotion]) -> Option<PromotionHint<'_>> {
    let amount = if amount.is_finite() && amount > 0.0 {
        amount
    } else {
        0.0
    };
    let current = pick_best_promotion(amount, rules).map_or(0, |found| to_cents(found.bonus));
    let amount_cents = to_cents(amount);

    let mut thresholds: Vec<i64> = rules
        .iter()
        .map(|rule| to_cents(rule.min_amount))
        .filter(|cents| *cents > amount_cents)
        .collect();
    thresholds.sort_unstable();
    thresholds.dedup();

    thresholds.into_iter().find_map(|cents| {
        let threshold = from_cents(cents);
        let found = pick_best_promotion(threshold, rules)?;
        (to_cents(found.bonus) > current).then(|| PromotionHint {
            rule: found.rule,
            threshold,
            need_more: from_cents(cents - amount_cents),
            bonus: found.bonus,
        })
    })
}

/// The quick-amount chips with the promotion thresholds folded in: deduped,
/// ascending, and inside `[min, max]`.
pub fn merge_quick_amounts(base: &[f64], thresholds: &[f64], min: f64, max: f64) -> Vec<f64> {
    let mut cents: Vec<i64> = base
        .iter()
        .chain(thresholds)
        .filter(|value| value.is_finite() && **value > 0.0)
        .map(|value| to_cents(*value))
        .filter(|cents| {
            let value = from_cents(*cents);
            value >= min && value <= max
        })
        .collect();
    cents.sort_unstable();
    cents.dedup();
    cents.into_iter().map(from_cents).collect()
}

/// Inside the promotion's window at `now_unix`. A bound that does not parse
/// is no bound, as on the web page.
pub fn is_in_window(promotion: &Promotion, now_unix: i64) -> bool {
    let bound = |raw: &Option<String>| {
        raw.as_deref()
            .map(str::trim)
            .filter(|raw| !raw.is_empty())
            .and_then(crate::pay::rfc3339_to_epoch)
    };
    if bound(&promotion.starts_at).is_some_and(|start| now_unix < start) {
        return false;
    }
    if bound(&promotion.ends_at).is_some_and(|end| now_unix >= end) {
        return false;
    }
    true
}

/// The promotions a top-up can earn right now: available and in window.
pub fn active_promotions(all: &[Promotion], now_unix: i64) -> Vec<Promotion> {
    all.iter()
        .filter(|promotion| promotion.available && is_in_window(promotion, now_unix))
        .cloned()
        .collect()
}

/// Whole dollars bare, anything else with two decimals: 100 → "100",
/// 12.5 → "12.50".
pub fn format_amount(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_owned();
    }
    let cents = to_cents(value);
    if cents % 100 == 0 {
        format!("{}", cents / 100)
    } else {
        format!("{:.2}", from_cents(cents))
    }
}

fn format_percent(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_owned();
    }
    let rounded = (value * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 {
        format!("{}", rounded as i64)
    } else {
        let text = format!("{rounded:.2}");
        text.trim_end_matches('0').to_owned()
    }
}

/// The promotion's one-line label: "充 $100 送 $10", "充 $100 送 10%（最高 $50）".
pub fn describe_promotion(rule: &Promotion, locale: PromoLocale) -> String {
    let min = format_amount(rule.min_amount);
    match rule.bonus_type {
        BonusType::Percent => {
            let percent = format_percent(rule.bonus_value);
            let cap = rule
                .max_bonus
                .filter(|cap| *cap > 0.0)
                .map(|cap| {
                    let cap = format_amount(cap);
                    match locale {
                        PromoLocale::Zh => format!("（最高 ${cap}）"),
                        PromoLocale::En => format!(" (up to ${cap})"),
                        PromoLocale::Ja => format!("（上限 ${cap}）"),
                    }
                })
                .unwrap_or_default();
            match locale {
                PromoLocale::Zh => format!("充 ${min} 送 {percent}%{cap}"),
                PromoLocale::En => format!("Top up ${min}+ and get {percent}% bonus{cap}"),
                PromoLocale::Ja => format!("${min} 以上のチャージで {percent}% 進呈{cap}"),
            }
        }
        BonusType::Fixed => {
            let bonus = format_amount(rule.bonus_value);
            match locale {
                PromoLocale::Zh => format!("充 ${min} 送 ${bonus}"),
                PromoLocale::En => format!("Top up ${min}+ and get ${bonus} bonus"),
                PromoLocale::Ja => format!("${min} 以上のチャージで ${bonus} 進呈"),
            }
        }
    }
}

/// `config.promotions`, one item at a time: a malformed promotion is
/// skipped rather than failing the whole config.
pub(crate) fn parse_promotions(value: Option<&serde_json::Value>) -> Vec<Promotion> {
    let Some(serde_json::Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| serde_json::from_value::<Promotion>(item.clone()).ok())
        .filter(|promotion| !promotion.id.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(id: &str, min: f64, bonus: f64) -> Promotion {
        Promotion {
            id: id.to_owned(),
            name: format!("promo {id}"),
            min_amount: min,
            bonus_type: BonusType::Fixed,
            bonus_value: bonus,
            available: true,
            ..Promotion::default()
        }
    }

    fn percent(id: &str, min: f64, value: f64, cap: Option<f64>) -> Promotion {
        Promotion {
            bonus_type: BonusType::Percent,
            bonus_value: value,
            max_bonus: cap,
            ..fixed(id, min, 0.0)
        }
    }

    #[test]
    fn bonus_is_zero_below_the_threshold_or_for_bad_input() {
        let rule = fixed("a", 100.0, 10.0);
        assert_eq!(compute_bonus(99.99, &rule), 0.0);
        assert_eq!(compute_bonus(0.0, &rule), 0.0);
        assert_eq!(compute_bonus(-5.0, &rule), 0.0);
        assert_eq!(compute_bonus(f64::NAN, &rule), 0.0);
        assert_eq!(compute_bonus(100.0, &fixed("z", 100.0, 0.0)), 0.0);
        assert_eq!(compute_bonus(100.0, &fixed("n", 100.0, -1.0)), 0.0);
    }

    #[test]
    fn fixed_bonus_is_flat_from_the_threshold() {
        let rule = fixed("a", 100.0, 10.0);
        assert_eq!(compute_bonus(100.0, &rule), 10.0);
        assert_eq!(compute_bonus(999.0, &rule), 10.0);
    }

    #[test]
    fn percent_bonus_rounds_down_to_the_cent_and_respects_the_cap() {
        let rule = percent("p", 100.0, 10.0, None);
        assert_eq!(compute_bonus(100.0, &rule), 10.0);
        assert_eq!(compute_bonus(333.33, &rule), 33.33);
        assert_eq!(compute_bonus(100.05, &rule), 10.0);
        assert_eq!(compute_bonus(200.0, &percent("h", 1.0, 12.5, None)), 25.0);

        let capped = percent("c", 100.0, 10.0, Some(50.0));
        assert_eq!(compute_bonus(400.0, &capped), 40.0);
        assert_eq!(compute_bonus(1000.0, &capped), 50.0);
    }

    #[test]
    fn bonus_never_overflows_the_credit_column() {
        let rule = percent("m", 1.0, 100.0, None);
        assert_eq!(compute_bonus(MAX_CREDIT_AMOUNT - 1.0, &rule), 1.0);
    }

    #[test]
    fn best_promotion_takes_the_biggest_bonus_then_sort_order_then_id() {
        let rules = vec![
            fixed("a", 100.0, 10.0),
            fixed("b", 500.0, 80.0),
            percent("c", 200.0, 10.0, None),
        ];
        assert!(pick_best_promotion(50.0, &rules).is_none());
        let found = pick_best_promotion(600.0, &rules).unwrap();
        assert_eq!((found.rule.id.as_str(), found.bonus), ("b", 80.0));
        let found = pick_best_promotion(300.0, &rules).unwrap();
        assert_eq!((found.rule.id.as_str(), found.bonus), ("c", 30.0));

        let mut x = fixed("x", 10.0, 5.0);
        x.sort_order = 2;
        let mut y = fixed("y", 10.0, 5.0);
        y.sort_order = 1;
        let mut a = fixed("a", 10.0, 5.0);
        a.sort_order = 1;
        assert_eq!(
            pick_best_promotion(10.0, &[x.clone(), y.clone()])
                .unwrap()
                .rule
                .id,
            "y"
        );
        assert_eq!(pick_best_promotion(10.0, &[x, y, a]).unwrap().rule.id, "a");
    }

    #[test]
    fn hint_points_at_the_nearest_threshold_that_pays_more() {
        let rules = vec![fixed("a", 100.0, 10.0), fixed("b", 500.0, 80.0)];
        let hint = next_promotion_hint(90.0, &rules).unwrap();
        assert_eq!(hint.rule.id, "a");
        assert_eq!(
            (hint.threshold, hint.need_more, hint.bonus),
            (100.0, 10.0, 10.0)
        );

        let hint = next_promotion_hint(120.0, &rules).unwrap();
        assert_eq!(hint.rule.id, "b");
        assert_eq!(hint.need_more, 380.0);

        assert!(next_promotion_hint(600.0, &rules).is_none());
        // A higher threshold that pays no more is no reason to top up more.
        let flat = vec![fixed("a", 100.0, 10.0), fixed("b", 200.0, 10.0)];
        assert!(next_promotion_hint(150.0, &flat).is_none());
    }

    #[test]
    fn quick_amounts_fold_in_thresholds_inside_the_range() {
        assert_eq!(
            merge_quick_amounts(
                &[10.0, 50.0, 100.0],
                &[100.0, 300.0, 5000.0, 0.5],
                1.0,
                1000.0
            ),
            vec![10.0, 50.0, 100.0, 300.0]
        );
        assert_eq!(
            merge_quick_amounts(&[10.0, f64::NAN, -1.0], &[99.5], 1.0, 1000.0),
            vec![10.0, 99.5]
        );
    }

    #[test]
    fn window_bounds_are_inclusive_start_exclusive_end() {
        // 2026-09-02T00:00:00Z
        let now = 1_788_307_200;
        let mut promotion = fixed("w", 10.0, 1.0);
        assert!(is_in_window(&promotion, now));
        promotion.starts_at = Some("2026-09-02T00:00:00Z".to_owned());
        assert!(is_in_window(&promotion, now));
        promotion.starts_at = Some("2026-09-02T00:00:01Z".to_owned());
        assert!(!is_in_window(&promotion, now));
        promotion.starts_at = None;
        promotion.ends_at = Some("2026-09-02T00:00:00.000Z".to_owned());
        assert!(!is_in_window(&promotion, now));
        promotion.ends_at = Some("2026-09-02T08:00:01+08:00".to_owned());
        assert!(is_in_window(&promotion, now));
        promotion.ends_at = Some("not a date".to_owned());
        assert!(is_in_window(&promotion, now));
    }

    #[test]
    fn active_promotions_drop_unavailable_and_expired_ones() {
        let now = 1_788_307_200;
        let mut used_up = fixed("u", 10.0, 1.0);
        used_up.available = false;
        let mut ended = fixed("e", 10.0, 1.0);
        ended.ends_at = Some("2026-09-01T00:00:00Z".to_owned());
        let active = active_promotions(&[used_up, ended, fixed("k", 10.0, 1.0)], now);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "k");
    }

    #[test]
    fn amounts_format_like_the_web_page() {
        assert_eq!(format_amount(100.0), "100");
        assert_eq!(format_amount(12.5), "12.50");
        assert_eq!(format_amount(0.1 + 0.2), "0.30");
        assert_eq!(format_percent(10.0), "10");
        assert_eq!(format_percent(12.5), "12.5");
    }

    #[test]
    fn descriptions_match_the_web_wording() {
        let flat = fixed("a", 100.0, 10.0);
        assert_eq!(describe_promotion(&flat, PromoLocale::Zh), "充 $100 送 $10");
        assert_eq!(
            describe_promotion(&flat, PromoLocale::En),
            "Top up $100+ and get $10 bonus"
        );
        assert_eq!(
            describe_promotion(&flat, PromoLocale::Ja),
            "$100 以上のチャージで $10 進呈"
        );

        let capped = percent("p", 100.0, 10.0, Some(50.0));
        assert_eq!(
            describe_promotion(&capped, PromoLocale::Zh),
            "充 $100 送 10%（最高 $50）"
        );
        assert_eq!(
            describe_promotion(&capped, PromoLocale::En),
            "Top up $100+ and get 10% bonus (up to $50)"
        );
        let uncapped = percent("q", 50.0, 12.5, None);
        assert_eq!(
            describe_promotion(&uncapped, PromoLocale::Zh),
            "充 $50 送 12.5%"
        );
    }

    #[test]
    fn locale_tags_map_to_promotion_languages() {
        assert_eq!(PromoLocale::from_locale("zh-CN"), PromoLocale::Zh);
        assert_eq!(PromoLocale::from_locale("ja"), PromoLocale::Ja);
        assert_eq!(PromoLocale::from_locale("en"), PromoLocale::En);
        assert_eq!(PromoLocale::from_locale(""), PromoLocale::En);
    }

    #[test]
    fn promotions_parse_leniently() {
        let value = serde_json::json!([
            {"id": "a", "name": "A", "description": null, "minAmount": 100, "bonusType": "fixed",
             "bonusValue": 10, "maxBonus": null, "sortOrder": 0, "startsAt": null,
             "endsAt": "2026-10-05T15:59:00.000Z", "available": true},
            {"id": "b", "name": "B", "minAmount": 50, "bonusType": "mystery", "bonusValue": 3},
            {"id": "", "name": "no id"},
            {"id": 7},
            "garbage"
        ]);
        let promotions = parse_promotions(Some(&value));
        assert_eq!(promotions.len(), 2);
        assert_eq!(
            promotions[0].ends_at.as_deref(),
            Some("2026-10-05T15:59:00.000Z")
        );
        assert_eq!(promotions[1].bonus_type, BonusType::Fixed);
        assert!(promotions[1].available);
        assert!(parse_promotions(None).is_empty());
        assert!(parse_promotions(Some(&serde_json::Value::Null)).is_empty());
    }
}
