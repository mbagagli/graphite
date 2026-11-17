//! Check explicit standard identifiers and requested length units.

use regex::Regex;

/// Check standard codes against the cited passages, never uncited context.
pub(super) fn unsupported_standards(line: &str, cited: &[&str]) -> Vec<String> {
    let mut missing = Vec::new();
    for (namespace, code_pattern) in [
        ("ASTM", r"(?i)\b[A-Z]\s*\d{3,}(?:[-:]\d+)?\b"),
        ("ISO", r"(?i)\bISO\s+\d+(?:[-:]\d+)*\b"),
        ("IEC", r"(?i)\bIEC\s+\d+(?:[-:]\d+)*\b"),
    ] {
        if !line.to_uppercase().contains(namespace) {
            continue;
        }
        let pattern = Regex::new(code_pattern).expect("static standard code");
        let supported: Vec<_> = cited
            .iter()
            .flat_map(|text| pattern.find_iter(text).map(|m| normalize_code(m.as_str())))
            .collect();
        for code in pattern.find_iter(line) {
            if !supported.contains(&normalize_code(code.as_str())) {
                missing.push(code.as_str().to_owned());
            }
        }
    }
    missing
}

/// Ignore OCR whitespace and case without changing identifier digits.
fn normalize_code(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

/// Require a recognized requested length unit without inventing a conversion.
pub(super) fn requested_unit_issue(question: &str, answer: &str) -> Option<String> {
    for aliases in [
        r"millimet(?:er|re)s?|mm",
        r"centimet(?:er|re)s?|cm",
        r"micromet(?:er|re)s?|microns?|[\x{00B5}\x{03BC}]m|um",
        r"nanomet(?:er|re)s?|nm",
        r"kilomet(?:er|re)s?|km",
        r"met(?:er|re)s?|m",
    ] {
        let request = Regex::new(&format!(r"(?i)\b(?:in|into|using)\s+({aliases})\b"))
            .expect("static requested unit");
        let Some(unit) = request.captures(question) else {
            continue;
        };
        let supplied = Regex::new(&format!(r"(?i)(?:^|[^\p{{L}}])(?:{aliases})\b"))
            .expect("static answer unit");
        if !supplied.is_match(answer) {
            return Some(format!(
                "The question requests {}. Give the value in that unit using only an unambiguous conversion of the supplied source quantity. Cite the source and show its original value. If conversion is unsupported, explicitly abstain instead of guessing.",
                &unit[1],
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_claim_requires_each_identifier_in_cited_evidence() {
        let line = "ASTM D2487 and D4318 [S1].";
        assert_eq!(unsupported_standards(line, &["ASTM D2487"]), vec!["D4318"]);
        assert!(unsupported_standards(line, &["ASTM D2487", "astm D 4318"]).is_empty());
        assert_eq!(
            unsupported_standards("ISO 9001 [S1]", &["ISO 14001"]),
            vec!["ISO 9001"]
        );
        assert!(unsupported_standards("A measurement of 100 [S1]", &["100"]).is_empty());
    }

    #[test]
    fn requested_units_accept_aliases_but_not_other_scales() {
        assert!(requested_unit_issue("Give length in centimetres.", "20 mm [S1]").is_some());
        assert!(requested_unit_issue("Give length in centimetres.", "2 cm (20 mm) [S1]").is_none());
        assert!(requested_unit_issue("Give length in meters.", "2 metres [S1]").is_none());
        assert!(requested_unit_issue("How many samples in the experiment?", "20 [S1]").is_none());
    }
}
