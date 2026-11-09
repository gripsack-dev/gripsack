//! The kernel's guarantees are Lean theorems over the Charon/Aeneas
//! translation (proofs/Kernel/, drift-checked against this crate by the
//! `proofs` gate). Behaviour that a theorem states for every input is not
//! re-tested here (plan/0057 D7); only functions without a theorem keep a
//! concrete guard.
use gripsack_kernel::GenerationId;

#[test]
fn generation_arithmetic_saturates_at_the_bounds() {
    assert_eq!(GenerationId::new(u64::MAX).checked_next(), None);
    assert_eq!(GenerationId::new(0).checked_previous(), None);
    assert_eq!(
        GenerationId::new(u64::MAX - 1).checked_next(),
        Some(GenerationId::new(u64::MAX))
    );
    assert_eq!(
        GenerationId::new(1).checked_previous(),
        Some(GenerationId::new(0))
    );
}

#[test]
fn interface_identities_compare_by_value() {
    use gripsack_kernel::{CmdId, DestId, Digest};
    assert_eq!(DestId::new(7), DestId::new(7));
    assert_ne!(DestId::new(7), DestId::new(8));
    let mut bytes = [0u8; gripsack_kernel::ids::DIGEST_BYTES];
    let a = Digest::from_bytes(bytes);
    bytes[31] = 1;
    assert_ne!(a, Digest::from_bytes(bytes));
    assert_eq!(a, Digest::from_bytes([0; 32]));
    assert_eq!(CmdId::new(u32::MAX).checked_next(), None);
    assert_eq!(CmdId::new(1).checked_next(), Some(CmdId::new(2)));
}
