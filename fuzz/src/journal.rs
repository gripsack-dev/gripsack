use crate::sandbox::Sandbox;
use gripsack_store::journal::{Entry, PriorSerde};

/// An optional NUL separates arbitrary marker bytes from arbitrary Entry JSON.
/// No marker representation is declared here: only reconcile parses run.json.
pub(crate) fn exercise(s: &Sandbox, input: &[u8]) {
    let (marker, entry_bytes) = match input.iter().position(|b| *b == 0) {
        Some(i) => (&input[..i], &input[i + 1..]),
        None => (input, input),
    };
    let parsed = serde_json::from_slice::<Entry>(entry_bytes);
    if let Ok(entry) = &parsed {
        let encoded = serde_json::to_vec(entry).unwrap();
        assert_eq!(&serde_json::from_slice::<Entry>(&encoded).unwrap(), entry);
    }
    let blob = gripsack_store::journal::store_prior_blob_in(s.cap(), b"bounded prior\n").unwrap();
    s.write("objects/prior-target", b"fixed target\n");
    let target = s.fixed("objects/prior-target").to_str().unwrap().to_owned();
    let dest = s.fixed("objects/destination").to_str().unwrap().to_owned();
    let mut entry = parsed.unwrap_or(Entry {
        dest: String::new(),
        prior: PriorSerde::File {
            hash: String::new(),
            mode: 0o600,
        },
        after: gripsack_store::journal::REMOVED.to_owned(),
    });
    // This is the sole transition from readonly arbitrary data to mutation data.
    entry.dest = dest;
    match &mut entry.prior {
        PriorSerde::File { hash, mode } => {
            *hash = blob;
            *mode &= 0o7777;
        }
        PriorSerde::Symlink {
            target: prior_target,
        } => *prior_target = target,
        PriorSerde::Absent => {}
    }
    s.write("journal/entry.json", &serde_json::to_vec(&entry).unwrap());
    s.write("journal/run.json", marker);
    // Fixed histories only. Numeric values inside marker bytes never select paths
    // in harness code, including negative, overflowing and fractional values.
    for n in [1, 2, 3] {
        gripsack_store::write_manifest(
            s.cap(),
            &gripsack_store::Generation {
                number: n,
                modules: Default::default(),
            },
        )
        .unwrap();
    }
    match entry_bytes.first().copied().unwrap_or(0) % 4 {
        0 => {}
        1 => gripsack_store::flip(s.cap(), s.home(), 1).unwrap(),
        2 => gripsack_store::flip(s.cap(), s.home(), 2).unwrap(),
        _ => gripsack_store::flip(s.cap(), s.home(), 3).unwrap(),
    }
    let _ = gripsack_store::reconcile(s.cap(), s.home());
    // The symlink referent is never a deployment destination.
    assert_eq!(
        std::fs::read(s.fixed("objects/prior-target")).unwrap(),
        b"fixed target\n"
    );
}
