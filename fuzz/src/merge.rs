use gripsack_exec::managed_blocks::ManagedBlockSet;
use std::path::Path;

pub(crate) fn exercise(input: &[u8]) {
    let Ok(text) = std::str::from_utf8(input) else {
        return;
    };
    let (existing, payload) = text.split_once('\0').unwrap_or((text, "replacement\n"));
    for module in ["m", "other"] {
        let Ok(blocks) = ManagedBlockSet::parse(existing, module) else {
            continue;
        };
        assert!(
            blocks
                .blocks()
                .iter()
                .all(|block| block.range.start < block.range.end
                    && block.range.end <= existing.len())
        );
        assert!(
            blocks
                .blocks()
                .windows(2)
                .all(|pair| pair[0].range.end <= pair[1].range.start)
        );
        // Independently collect only the source's unowned slices.
        let mut foreign = String::new();
        let mut cursor = 0;
        for block in blocks.blocks() {
            foreign.push_str(&existing[cursor..block.range.start]);
            cursor = block.range.end;
        }
        foreign.push_str(&existing[cursor..]);
        if let Some(removed) = blocks.remove() {
            assert_eq!(removed, foreign)
        }
        for dest in ["config.sh", "config.html", "config.jsonc", ".vimrc"] {
            if let Ok(output) = blocks.upsert(module, Path::new(dest), None, payload, 0o644) {
                let output_blocks = ManagedBlockSet::parse(&output, module).unwrap();
                assert_eq!(output_blocks.blocks().len(), 1);
                if !blocks.is_empty() {
                    assert_eq!(output_blocks.remove().unwrap(), foreign)
                }
            }
        }
    }
    let digest = gripsack_store::hash::hex_sha256(input);
    let newline = if input.first().is_some_and(|byte| byte & 1 != 0) {
        "\r\n"
    } else {
        "\n"
    };
    let copies = input.first().map_or(1, |byte| (byte % 4 + 1) as usize);
    let foreign = format!("foreign-{digest}{newline}");
    let first = ManagedBlockSet::parse(&foreign, "m")
        .unwrap()
        .upsert("m", Path::new("config.sh"), None, &digest, 0o644)
        .unwrap();
    let duplicated = first.repeat(copies);
    let parsed = ManagedBlockSet::parse(&duplicated, "m").unwrap();
    assert_eq!(parsed.remove().unwrap(), foreign.repeat(copies));
    let repaired = parsed
        .upsert("m", Path::new("config.sh"), None, &digest, 0o644)
        .unwrap();
    let parsed = ManagedBlockSet::parse(&repaired, "m").unwrap();
    assert_eq!(parsed.blocks().len(), 1);
    assert_eq!(
        parsed
            .upsert("m", Path::new("config.sh"), None, &digest, 0o644)
            .unwrap(),
        repaired
    );
}
