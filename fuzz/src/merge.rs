use gripsack_exec::template::{extract_block, find_blocks, marker_sha, remove_block, upsert_block};
use std::path::Path;

pub(crate) fn exercise(input: &[u8]) {
    let Ok(text) = std::str::from_utf8(input) else {
        return;
    };
    let (existing, payload) = text.split_once('\0').unwrap_or((text, "replacement\n"));
    for module in ["m", "other"] {
        let blocks = find_blocks(existing, module);
        assert!(
            blocks
                .iter()
                .all(|(open, close)| open < close && *close < existing.lines().count())
        );
        assert!(blocks.windows(2).all(|pair| pair[0].1 < pair[1].0));
        let _ = marker_sha(existing, module);
        let _ = extract_block(existing, module);
        let _ = remove_block(existing, module);
        for dest in ["config.sh", "config.html", "config.jsonc", ".vimrc"] {
            if let Ok(out) = upsert_block(existing, module, Path::new(dest), None, payload) {
                let _ = find_blocks(&out, module);
                let _ = extract_block(&out, module);
                let _ = marker_sha(&out, module);
                let _ = remove_block(&out, module);
                let _ = upsert_block(&out, module, Path::new(dest), Some("#!"), payload);
            }
        }
    }
    // Input-derived controlled grammar: marker-like arbitrary input is exercised
    // above; here the expected external content is independently knowable.
    let digest = gripsack_store::hash::hex_sha256(input);
    let eol = if input.first().is_some_and(|byte| byte & 1 != 0) {
        "\r\n"
    } else {
        "\n"
    };
    let copies = input.first().map_or(1, |byte| (byte % 4 + 1) as usize);
    let foreign = format!("foreign-{digest}{eol}");
    let first = upsert_block(&foreign, "m", Path::new("config.sh"), None, &digest).unwrap();
    let duplicated = first.repeat(copies);
    let repaired = upsert_block(&duplicated, "m", Path::new("config.sh"), None, &digest).unwrap();
    assert_eq!(find_blocks(&repaired, "m").len(), 1);
    assert_eq!(
        upsert_block(&repaired, "m", Path::new("config.sh"), None, &digest).unwrap(),
        repaired
    );
    let removed = remove_block(&duplicated, "m").unwrap();
    assert!(find_blocks(&removed, "m").is_empty());
    // Removal trims trailing blanks, not internal unowned separator lines.
    let expected = format!("{foreign}{eol}").repeat(copies - 1) + &foreign;
    assert_eq!(removed, expected);
}
