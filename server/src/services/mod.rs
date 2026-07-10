pub mod python;
pub mod nodejs;

/// Process-was-killed-by-SIGSYS sentinel values.
/// SIGSYS = signal 31; raw wait status = 128 + 31 = 159; negative-signal = -31.
pub(crate) const SIGSYS_RAW: i32 = 159;
pub(crate) const SIGSYS_NEG: i32 = -31;

pub(crate) fn is_seccomp_violation(exit_code: i32) -> bool {
    exit_code == SIGSYS_RAW || exit_code == SIGSYS_NEG
}
