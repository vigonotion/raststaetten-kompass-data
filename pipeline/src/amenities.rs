//! Amenity rules (heuristics §6.4): shower, fast charger, open 24 h.

use crate::osm::model::Tags;
use crate::params::{FAST_CHARGER_MIN_KW, OPEN_24H_SPELLINGS};

/// Parses one power value part into kW: number (decimal point or comma) followed by an optional
/// unit `kW`/`W`/`kVA`/`MW` (case-insensitive, with or without a space). Without a unit,
/// values >= 1000 are watts, otherwise kW. Anything else is unparseable (`None`).
pub fn parse_power_part_kw(part: &str) -> Option<f64> {
    let s = part.trim();
    let num_end = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(num_end);
    if num.is_empty() {
        return None;
    }
    let value: f64 = num.replace(',', ".").parse().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    match unit.trim().to_ascii_lowercase().as_str() {
        "kw" | "kva" => Some(value),
        "w" => Some(value / 1000.0),
        "mw" => Some(value * 1000.0),
        "" => Some(if value >= 1000.0 {
            value / 1000.0
        } else {
            value
        }),
        _ => None,
    }
}

/// Maximum over all `;`-separated parts, in kW. `None` if no part is parseable.
pub fn parse_power_kw(value: &str) -> Option<f64> {
    value
        .split(';')
        .filter_map(parse_power_part_kw)
        .fold(None, |acc, v| Some(acc.map_or(v, |a: f64| a.max(v))))
}

/// A charging station counts as fast charger: `socket:type2_combo` as an integer >= 1, or a
/// `socket:*:output` / `maxpower` value >= 50 kW.
pub fn is_fast_charger(tags: &Tags) -> bool {
    for (k, v) in &tags.0 {
        if k == "socket:type2_combo" {
            if let Ok(n) = v.trim().parse::<u32>() {
                if n >= 1 {
                    return true;
                }
            }
        } else if (k == "maxpower" || (k.starts_with("socket:") && k.ends_with(":output")))
            && parse_power_kw(v).is_some_and(|kw| kw >= FAST_CHARGER_MIN_KW)
        {
            return true;
        }
    }
    false
}

/// `opening_hours` is one of the 24/7 spellings (heuristics §6.4): compared without any
/// whitespace and case-insensitively.
pub fn is_open_24_7(tags: &Tags) -> bool {
    tags.get("opening_hours").is_some_and(|v| {
        let s: String = v
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect();
        OPEN_24H_SPELLINGS.contains(&s.as_str())
    })
}

pub fn has_shower_yes(tags: &Tags) -> bool {
    tags.get("shower").is_some_and(|v| v.trim() == "yes")
}

/// `amenity` is a gastronomy value (`fast_food`, `restaurant`, `cafe`).
pub fn is_gastronomy(tags: &Tags) -> bool {
    matches!(
        tags.get("amenity"),
        Some("fast_food") | Some("restaurant") | Some("cafe")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(kv: &[(&str, &str)]) -> Tags {
        Tags(
            kv.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    #[test]
    fn power_units() {
        assert_eq!(parse_power_part_kw("50 kW"), Some(50.0));
        assert_eq!(parse_power_part_kw("50kw"), Some(50.0));
        assert_eq!(parse_power_part_kw("150 KW"), Some(150.0));
        assert_eq!(parse_power_part_kw("22000 W"), Some(22.0));
        assert_eq!(parse_power_part_kw("22000W"), Some(22.0));
        assert_eq!(parse_power_part_kw("43 kVA"), Some(43.0));
        assert_eq!(parse_power_part_kw("0.35 MW"), Some(350.0));
        assert_eq!(parse_power_part_kw("0,35 mw"), Some(350.0));
        assert_eq!(parse_power_part_kw("22,5 kW"), Some(22.5));
    }

    #[test]
    fn power_unitless_rule() {
        assert_eq!(parse_power_part_kw("50"), Some(50.0));
        assert_eq!(parse_power_part_kw("999"), Some(999.0));
        assert_eq!(parse_power_part_kw("1000"), Some(1.0)); // watts
        assert_eq!(parse_power_part_kw("50000"), Some(50.0)); // watts
        assert_eq!(parse_power_part_kw("350"), Some(350.0));
    }

    #[test]
    fn power_unparseable_and_lists() {
        assert_eq!(parse_power_part_kw(""), None);
        assert_eq!(parse_power_part_kw("fast"), None);
        assert_eq!(parse_power_part_kw("50 kW max"), None);
        assert_eq!(parse_power_part_kw("-5"), None);
        assert_eq!(parse_power_part_kw("1.2.3"), None);
        assert_eq!(parse_power_kw("11 kW;150 kW;22 kW"), Some(150.0));
        assert_eq!(parse_power_kw("abc;22"), Some(22.0));
        assert_eq!(parse_power_kw("abc;def"), None);
        assert_eq!(parse_power_kw("22;50000 W"), Some(50.0));
    }

    #[test]
    fn fast_charger_rules() {
        assert!(is_fast_charger(&tags(&[("socket:type2_combo", "2")])));
        assert!(is_fast_charger(&tags(&[("socket:type2_combo", " 1 ")])));
        assert!(!is_fast_charger(&tags(&[("socket:type2_combo", "yes")])));
        assert!(!is_fast_charger(&tags(&[("socket:type2_combo", "0")])));
        assert!(is_fast_charger(&tags(&[(
            "socket:chademo:output",
            "50 kW"
        )])));
        assert!(is_fast_charger(&tags(&[("maxpower", "150000")])));
        assert!(is_fast_charger(&tags(&[("socket:ccs:output", "0,15 MW")])));
        assert!(!is_fast_charger(&tags(&[("socket:type2:output", "22 kW")])));
        assert!(!is_fast_charger(&tags(&[("maxpower", "49.9 kW")])));
        assert!(!is_fast_charger(&tags(&[(
            "socket:type2_combo:output",
            "unknown"
        )])));
        assert!(!is_fast_charger(&tags(&[])));
    }

    #[test]
    fn simple_flags() {
        assert!(is_open_24_7(&tags(&[("opening_hours", " 24/7 ")])));
        for v in [
            "24/7",
            "Mo-Su 00:00-24:00",
            "mo-su 0:00-24:00",
            " Mo-Su  00:00 - 24:00 ",
            "00:00-24:00",
            "24 / 7",
        ] {
            assert!(is_open_24_7(&tags(&[("opening_hours", v)])), "{v}");
        }
        for v in ["Mo-Fr 00:00-24:00", "Mo-Su 06:00-22:00", "24/7; PH off", ""] {
            assert!(!is_open_24_7(&tags(&[("opening_hours", v)])), "{v}");
        }
        assert!(has_shower_yes(&tags(&[("shower", "yes")])));
        assert!(!has_shower_yes(&tags(&[("shower", "no")])));
        assert!(is_gastronomy(&tags(&[("amenity", "cafe")])));
        assert!(!is_gastronomy(&tags(&[("amenity", "fuel")])));
    }
}
