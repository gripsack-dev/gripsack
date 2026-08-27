//! difflib's SequenceMatcher.ratio, ported: the did-you-mean
//! suggestions in fixtures pin difflib's exact answers, so the
//! Ratcliff-Obershelp match count is reproduced, not approximated.

/// Longest contiguous matching block; returns (start_a, start_b, size).
fn longest_match(a: &[char], b: &[char]) -> (usize, usize, usize) {
    // b2j: value → positions in b
    let mut b2j: std::collections::HashMap<char, Vec<usize>> = std::collections::HashMap::new();
    for (i, &c) in b.iter().enumerate() {
        b2j.entry(c).or_default().push(i);
    }
    let mut best = (0, 0, 0);
    let mut matchlen_at = vec![0usize; b.len() + 1]; // match length ending at b pos
    for (i, &c) in a.iter().enumerate() {
        let mut new_matchlen = vec![0usize; b.len() + 1];
        if let Some(positions) = b2j.get(&c) {
            for &j in positions {
                let k = matchlen_at[j] + 1;
                new_matchlen[j + 1] = k;
                if k > best.2 {
                    best = (i + 1 - k, j + 1 - k, k);
                }
            }
        }
        matchlen_at = new_matchlen;
    }
    best
}

fn count_matches(a: &[char], b: &[char]) -> usize {
    let (i, j, size) = longest_match(a, b);
    if size == 0 {
        return 0;
    }
    size + count_matches(&a[..i], &b[..j]) + count_matches(&a[i + size..], &b[j + size..])
}

/// difflib ratio: 2*M / (|a| + |b|).
pub fn ratio(a: &str, b: &str) -> f64 {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let total = ac.len() + bc.len();
    if total == 0 {
        return 1.0;
    }
    2.0 * count_matches(&ac, &bc) as f64 / total as f64
}

/// get_close_matches(word, known, n=1, cutoff=0.7): the best match at
/// or above the cutoff (fixtures pin unambiguous winners).
pub fn suggest<'a>(key: &str, known: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    known
        .into_iter()
        .map(|c| (c, ratio(key, c)))
        .filter(|(_, r)| *r >= 0.7)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(c, _)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_suggestions_hold() {
        assert_eq!(
            suggest("scrollof", ["scrolloff", "mouse"]),
            Some("scrolloff")
        );
        assert_eq!(
            suggest("scrollof", ["scrolloff", "scroll-lines"]),
            Some("scrolloff")
        );
        assert_eq!(suggest("zzzz", ["scrolloff"]), None);
    }
}
