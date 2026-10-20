//! `data/brands.json` parsing and validation (docs/schema.md §6).

use crate::error::{Error, Result};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

const CATEGORIES: [&str; 6] = ["food", "coffee", "fuel", "charging", "shop", "other"];

/// Parsed brands.json entry; `category` and `simple_icons_slug` are validated but only used by the app.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Brand {
    pub id: String,
    pub display_name: String,
    pub category: String,
    pub match_brand: Vec<String>,
    pub match_name: Vec<String>,
    pub simple_icons_slug: Option<String>,
}

pub fn is_kebab_case(s: &str) -> bool {
    !s.is_empty()
        && s.split('-').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

fn string_array(v: &Value, ctx: &str, errors: &mut Vec<String>) -> Vec<String> {
    match v.as_array() {
        Some(a) => a
            .iter()
            .filter_map(|x| match x.as_str() {
                Some(s) => Some(s.to_string()),
                None => {
                    errors.push(format!("{ctx}: entries must be strings"));
                    None
                }
            })
            .collect(),
        None => {
            errors.push(format!("{ctx}: must be an array of strings"));
            Vec::new()
        }
    }
}

/// Parses and validates brands JSON text. All problems are collected.
pub fn parse(text: &str, origin: &str) -> Result<Vec<Brand>> {
    let v: Value =
        serde_json::from_str(text).map_err(|e| Error::Json(format!("cannot parse {origin}"), e))?;
    let Some(arr) = v.as_array() else {
        return Err(Error::Data(format!("{origin}: top level must be an array")));
    };
    let mut errors = Vec::new();
    let mut brands: Vec<Brand> = Vec::new();
    for (i, b) in arr.iter().enumerate() {
        let ctx = format!("{origin}[{i}]");
        let Some(obj) = b.as_object() else {
            errors.push(format!("{ctx}: must be an object"));
            continue;
        };
        let id = obj.get("id").and_then(Value::as_str).unwrap_or("");
        let ctx = if id.is_empty() {
            ctx
        } else {
            format!("{ctx} ({id})")
        };
        if !is_kebab_case(id) {
            errors.push(format!("{ctx}: id must be kebab-case"));
        } else if brands.iter().any(|x| x.id == id) {
            errors.push(format!("{ctx}: duplicate id"));
        }
        let display_name = obj.get("displayName").and_then(Value::as_str).unwrap_or("");
        if display_name.trim().is_empty() {
            errors.push(format!("{ctx}: displayName must be a non-empty string"));
        }
        let category = obj.get("category").and_then(Value::as_str).unwrap_or("");
        if !CATEGORIES.contains(&category) {
            errors.push(format!("{ctx}: category must be one of {CATEGORIES:?}"));
        }
        let (mut match_brand, mut match_name) = (Vec::new(), Vec::new());
        match obj.get("match").and_then(Value::as_object) {
            Some(m) => {
                match m.get("brand") {
                    Some(x) => {
                        match_brand = string_array(x, &format!("{ctx}.match.brand"), &mut errors)
                    }
                    None => errors.push(format!("{ctx}: match.brand missing")),
                }
                match m.get("name") {
                    Some(x) => {
                        match_name = string_array(x, &format!("{ctx}.match.name"), &mut errors)
                    }
                    None => errors.push(format!("{ctx}: match.name missing")),
                }
            }
            None => errors.push(format!("{ctx}: match must be an object")),
        }
        let slug = match obj.get("simpleIconsSlug") {
            Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            _ => {
                errors.push(format!("{ctx}: simpleIconsSlug must be a string or null"));
                None
            }
        };
        brands.push(Brand {
            id: id.to_string(),
            display_name: display_name.to_string(),
            category: category.to_string(),
            match_brand,
            match_name,
            simple_icons_slug: slug,
        });
    }
    if errors.is_empty() {
        Ok(brands)
    } else {
        Err(Error::Validation(errors))
    }
}

pub fn load(path: &str) -> Result<Vec<Brand>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| Error::Io(format!("cannot read brands file {path}"), e))?;
    parse(&text, path)
}

/// Normalization for all brand comparisons (heuristics §5): trim, collapse whitespace,
/// Unicode lowercase, typographic apostrophes to `'`.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for word in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        for c in word.chars() {
            match c {
                '\u{2019}' | '\u{02BC}' => out.push('\''),
                _ => out.extend(c.to_lowercase()),
            }
        }
    }
    out
}

/// Result of matching the `brand` tag.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BrandTagMatch {
    /// Indices (into the brands list) of matched brands.
    pub brands: BTreeSet<usize>,
    /// Trimmed parts of the tag that matched nothing.
    pub unmatched: Vec<String>,
}

/// Brand matching against `data/brands.json` (heuristics §5).
pub struct Matcher {
    by_brand_value: HashMap<String, Vec<usize>>,
    /// (normalized pattern, brand index)
    name_patterns: Vec<(String, usize)>,
}

impl Matcher {
    pub fn new(brands: &[Brand]) -> Matcher {
        let mut by_brand_value: HashMap<String, Vec<usize>> = HashMap::new();
        let mut name_patterns = Vec::new();
        for (i, b) in brands.iter().enumerate() {
            for v in &b.match_brand {
                let list = by_brand_value.entry(normalize(v)).or_default();
                if !list.contains(&i) {
                    list.push(i);
                }
            }
            for p in &b.match_name {
                let n = normalize(p);
                if !n.is_empty() {
                    name_patterns.push((n, i));
                }
            }
        }
        Matcher {
            by_brand_value,
            name_patterns,
        }
    }

    /// `brand` tag: split at `;`, every part compared for equality (after normalization).
    pub fn match_brand_tag(&self, tag: &str) -> BrandTagMatch {
        let mut r = BrandTagMatch::default();
        for part in tag.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            match self.by_brand_value.get(&normalize(part)) {
                Some(ids) => r.brands.extend(ids.iter().copied()),
                None => r.unmatched.push(part.to_string()),
            }
        }
        r
    }

    /// `name`: whole-word-sequence patterns; of overlapping matches only the longest counts.
    pub fn match_name(&self, name: &str) -> BTreeSet<usize> {
        let text = normalize(name);
        // (start, end, brand) in byte offsets of the normalized text.
        let mut hits: Vec<(usize, usize, usize)> = Vec::new();
        for (pat, brand) in &self.name_patterns {
            for (start, _) in text.match_indices(pat.as_str()) {
                let end = start + pat.len();
                let before_ok = text[..start]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !c.is_alphanumeric());
                let after_ok = text[end..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_alphanumeric());
                if before_ok && after_ok {
                    hits.push((start, end, *brand));
                }
            }
        }
        let mut out = BTreeSet::new();
        for &(s, e, b) in &hits {
            let dominated = hits
                .iter()
                .any(|&(s2, e2, _)| s2 < e && s < e2 && (e2 - s2) > (e - s));
            if !dominated {
                out.insert(b);
            }
        }
        out
    }

    /// Operator fallback for charging stations (heuristics §5, P-04): the `operator` tag is
    /// matched like a `brand` tag (equality per `;` part), then by whole-word name patterns.
    pub fn match_operator(&self, operator: &str) -> BTreeSet<usize> {
        let mut r = self.match_brand_tag(operator).brands;
        if r.is_empty() {
            r = self.match_name(operator);
        }
        r
    }

    /// Complete matching of one object: `brand` tag first, `name` only if the brand tag is
    /// missing or nothing in it matched. `use_name = false` for the services object itself.
    pub fn match_object(
        &self,
        brand_tag: Option<&str>,
        name: Option<&str>,
        use_name: bool,
    ) -> BrandTagMatch {
        let mut r = match brand_tag {
            Some(t) => self.match_brand_tag(t),
            None => BrandTagMatch::default(),
        };
        if r.brands.is_empty() && use_name {
            if let Some(n) = name {
                r.brands = self.match_name(n);
            }
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_brands_file_is_valid() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/brands.json");
        let b = load(path).expect("brands.json valid");
        assert!(!b.is_empty());
    }

    #[test]
    fn rejects_bad_input() {
        let bad = r#"[{"id":"Bad_Id","displayName":"","category":"x","match":{"brand":["a"]}},
                      {"id":"a","displayName":"A","category":"food","match":{"brand":[],"name":[]},"simpleIconsSlug":null},
                      {"id":"a","displayName":"A","category":"food","match":{"brand":[],"name":[]},"simpleIconsSlug":null}]"#;
        match parse(bad, "t") {
            Err(Error::Validation(e)) => assert!(e.len() >= 5, "{e:?}"),
            other => panic!("{other:?}"),
        }
        assert!(parse("{}", "t").is_err());
        assert!(is_kebab_case("burger-king") && !is_kebab_case("-a") && !is_kebab_case("A"));
    }

    fn test_brands() -> Vec<Brand> {
        let mk = |id: &str, b: &[&str], n: &[&str]| Brand {
            id: id.into(),
            display_name: id.into(),
            category: "other".into(),
            match_brand: b.iter().map(|x| x.to_string()).collect(),
            match_name: n.iter().map(|x| x.to_string()).collect(),
            simple_icons_slug: None,
        };
        vec![
            mk("shell", &["Shell"], &["Shell"]),
            mk("shell-recharge", &["Shell Recharge"], &["Shell Recharge"]),
            mk("star", &["Star"], &[]),
            mk("starbucks", &["Starbucks"], &["Starbucks"]),
            mk("jet", &["JET"], &["JET"]),
            mk("mcdonalds", &["McDonald's"], &["McDonald's", "McDonalds"]),
            mk("bk", &["Burger King"], &["Burger King"]),
        ]
    }

    fn names(m: &Matcher, name: &str) -> Vec<usize> {
        m.match_name(name).into_iter().collect()
    }

    #[test]
    fn normalization() {
        assert_eq!(
            normalize("  McDonald\u{2019}s \t Rest  "),
            "mcdonald's rest"
        );
        assert_eq!(normalize("MÖVENPICK"), "mövenpick");
    }

    #[test]
    fn brand_tag_equality_and_lists() {
        let m = Matcher::new(&test_brands());
        let r = m.match_brand_tag("shell; Burger  King;Foo");
        assert_eq!(r.brands.into_iter().collect::<Vec<_>>(), vec![0, 6]);
        assert_eq!(r.unmatched, vec!["Foo".to_string()]);
        // Equality, not substring.
        assert!(m.match_brand_tag("Shell Station").brands.is_empty());
        assert_eq!(
            m.match_brand_tag("McDonald\u{2019}s")
                .brands
                .into_iter()
                .collect::<Vec<_>>(),
            vec![5]
        );
    }

    #[test]
    fn name_whole_word_and_longest_match() {
        let m = Matcher::new(&test_brands());
        assert_eq!(names(&m, "Shell Recharge Hamburg"), vec![1]); // not 0
        assert_eq!(names(&m, "Shell Station"), vec![0]);
        assert_eq!(names(&m, "Starbucks Coffee"), vec![3]); // "Star" has no name pattern anyway
        assert!(names(&m, "Jetpack Tankstelle").is_empty());
        assert_eq!(names(&m, "JET Tankstelle"), vec![4]);
        assert_eq!(names(&m, "Autohof (JET)"), vec![4]);
        assert!(names(&m, "Rasthof-Shellfish").is_empty());
        assert_eq!(names(&m, "Rasthof-Shell"), vec![0]);
        assert_eq!(names(&m, "Shell & Burger King"), vec![0, 6]);
        assert_eq!(names(&m, "McDonald\u{2019}s"), vec![5]);
    }

    #[test]
    fn starbucks_does_not_match_star_by_name() {
        let mut b = test_brands();
        b[2].match_name = vec!["Star".into()];
        let m = Matcher::new(&b);
        assert_eq!(names(&m, "Starbucks"), vec![3]);
        assert_eq!(names(&m, "Star Tankstelle"), vec![2]);
    }

    #[test]
    fn operator_matches_like_brand_then_name() {
        let m = Matcher::new(&test_brands());
        assert_eq!(
            m.match_operator("Shell; Foo")
                .into_iter()
                .collect::<Vec<_>>(),
            vec![0]
        );
        assert_eq!(
            m.match_operator("Shell Recharge Solutions GmbH")
                .into_iter()
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert!(m.match_operator("Stadtwerke Foo").is_empty());
    }

    #[test]
    fn name_only_used_without_brand_match() {
        let m = Matcher::new(&test_brands());
        let r = m.match_object(Some("Shell"), Some("JET"), true);
        assert_eq!(r.brands.into_iter().collect::<Vec<_>>(), vec![0]);
        let r = m.match_object(Some("Unknown"), Some("JET"), true);
        assert_eq!(r.brands.into_iter().collect::<Vec<_>>(), vec![4]);
        assert_eq!(r.unmatched, vec!["Unknown".to_string()]);
        let r = m.match_object(None, Some("JET"), false);
        assert!(r.brands.is_empty());
    }
}
