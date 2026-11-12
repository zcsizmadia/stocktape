//! Number formatting for display.
//!
//! US-style for now (`1,234.56`); locale-aware formats per market come with
//! global markets (#10).

/// Decimal places used for a price of this size.
///
/// - 100 000 and above: none (`101,284`)
/// - 1 and above: two (`238.41`)
/// - below 1: four (`0.4521`)
#[must_use]
pub fn price_decimals(price: f64) -> usize {
    let magnitude = price.abs();
    if magnitude >= 100_000.0 {
        0
    } else if magnitude >= 1.0 {
        2
    } else {
        4
    }
}

/// Formats a price with thousands separators, e.g. `6,842.17`.
///
/// Non-finite values format as `—`.
///
/// ```
/// use stocktape_core::format::format_price;
/// assert_eq!(format_price(6842.17), "6,842.17");
/// assert_eq!(format_price(101_284.4), "101,284");
/// assert_eq!(format_price(0.45213), "0.4521");
/// ```
#[must_use]
pub fn format_price(price: f64) -> String {
    format_with(price, price_decimals(price), false)
}

/// Formats a change with an explicit sign, using the precision of `price`,
/// e.g. `+2.17` or `−3.44` (with a true minus sign).
///
/// ```
/// use stocktape_core::format::format_change;
/// assert_eq!(format_change(-3.44, 512.08), "−3.44");
/// assert_eq!(format_change(1912.0, 101_284.0), "+1,912");
/// ```
#[must_use]
pub fn format_change(change: f64, price: f64) -> String {
    format_with(change, price_decimals(price), true)
}

/// Formats a percentage change with an explicit sign and two decimals, e.g.
/// `+0.92%`.
///
/// ```
/// use stocktape_core::format::format_percent;
/// assert_eq!(format_percent(0.9213), "+0.92%");
/// assert_eq!(format_percent(-2.851), "−2.85%");
/// ```
#[must_use]
pub fn format_percent(percent: f64) -> String {
    let mut s = format_with(percent, 2, true);
    if percent.is_finite() {
        s.push('%');
    }
    s
}

fn format_with(value: f64, decimals: usize, signed: bool) -> String {
    if !value.is_finite() {
        return "—".to_owned();
    }
    let fixed = format!("{:.*}", decimals, value.abs());
    let (int_part, frac_part) = fixed.split_once('.').unwrap_or((&fixed, ""));
    let is_zero = fixed.bytes().all(|b| b == b'0' || b == b'.');

    let mut out = String::with_capacity(fixed.len() + fixed.len() / 3 + 2);
    if !is_zero && value < 0.0 {
        out.push('−');
    } else if signed && !is_zero {
        out.push('+');
    }
    let digits = int_part.len();
    for (i, c) in int_part.chars().enumerate() {
        if i > 0 && (digits - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if !frac_part.is_empty() {
        out.push('.');
        out.push_str(frac_part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn prices() {
        assert_eq!(format_price(238.41), "238.41");
        assert_eq!(format_price(47_215.3), "47,215.30");
        assert_eq!(format_price(1_000_000.0), "1,000,000");
        assert_eq!(format_price(999.999), "1,000.00");
        assert_eq!(format_price(-37.63), "−37.63");
        assert_eq!(format_price(0.0), "0.0000");
        assert_eq!(format_price(f64::NAN), "—");
    }

    #[test]
    fn changes() {
        assert_eq!(format_change(2.17, 238.41), "+2.17");
        assert_eq!(format_change(-0.004, 238.41), "0.00");
        assert_eq!(format_change(0.0, 238.41), "0.00");
        assert_eq!(format_change(-12.64, 431.22), "−12.64");
        assert_eq!(format_change(f64::INFINITY, 1.0), "—");
    }

    #[test]
    fn percents() {
        assert_eq!(format_percent(2.66), "+2.66%");
        assert_eq!(format_percent(0.0), "0.00%");
        assert_eq!(format_percent(f64::NAN), "—");
    }

    proptest! {
        #[test]
        fn separators_are_well_placed(value in -1e12f64..1e12) {
            let s = format_price(value);
            let digits = s.trim_start_matches('−');
            let int_part = digits.split('.').next().unwrap();
            let groups: Vec<&str> = int_part.split(',').collect();
            prop_assert!(!groups[0].is_empty() && groups[0].len() <= 3);
            for g in &groups[1..] {
                prop_assert_eq!(g.len(), 3);
            }
            let plain: String = s.chars().filter(|c| *c != ',').collect::<String>().replace('−', "-");
            let parsed: f64 = plain.parse().unwrap();
            let tolerance = 0.5 * 10f64.powi(-(price_decimals(value) as i32)) + 1e-9 * value.abs();
            prop_assert!((parsed - value).abs() <= tolerance);
        }
    }
}
