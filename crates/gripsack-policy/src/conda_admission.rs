//! Advisory Conda response admission, not publication authority. The adapter
//! supplies the supervised terminal outcome and borrowed decoded fields. Frame
//! version/length/EOF and native lifecycle observations are outside this model;
//! archive, receipt and installed-tree truth require independent core validation.

/// A reaped normal exit retains its actual code; all other native outcomes
/// (signal, deadline, output limit, cleanup failure) are interrupted terminals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeTerminal {
    Exited(i32),
    Interrupted,
}

/// Materialization cannot be confused with a solve or import. Its digest binds
/// the complete frozen closure INCLUDING materializer policy, not package names.
#[derive(Debug, Clone, Copy)]
pub enum OperationContext<'a> {
    Resolve,
    ImportPixi,
    Materialize {
        closure_digest: &'a str,
        final_prefix: &'a str,
    },
}

/// Views borrow the original request/response; admission never copies fields.
#[derive(Debug, Clone, Copy)]
pub struct ResponseBinding<'a> {
    pub attempt: u64,
    pub platform: &'a str,
    pub context: OperationContext<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionRefusal {
    NativeTerminal,
    Attempt,
    Platform,
    Operation,
    Closure,
    FinalPrefix,
}

/// Used before decoding too: a valid frame cannot erase a failed terminal.
pub fn terminal_succeeded(terminal: NativeTerminal) -> bool {
    matches!(terminal, NativeTerminal::Exited(0))
}

/// Total exact-binding contract over arbitrary decoded byte strings. No helper
/// success flag or precondition assumes the conclusion. Refusal never grants
/// publication permission; Ok admits only advisory data for independent checks.
pub fn admit_response(
    terminal: NativeTerminal,
    expected: ResponseBinding<'_>,
    actual: ResponseBinding<'_>,
) -> Result<(), AdmissionRefusal> {
    if !terminal_succeeded(terminal) {
        return Err(AdmissionRefusal::NativeTerminal);
    }
    if expected.attempt != actual.attempt {
        return Err(AdmissionRefusal::Attempt);
    }
    if expected.platform != actual.platform {
        return Err(AdmissionRefusal::Platform);
    }
    match (expected.context, actual.context) {
        (OperationContext::Resolve, OperationContext::Resolve) => Ok(()),
        (OperationContext::ImportPixi, OperationContext::ImportPixi) => Ok(()),
        (
            OperationContext::Materialize {
                closure_digest: ed,
                final_prefix: ep,
            },
            OperationContext::Materialize {
                closure_digest: ad,
                final_prefix: ap,
            },
        ) => {
            if ed != ad {
                Err(AdmissionRefusal::Closure)
            } else if ep != ap {
                Err(AdmissionRefusal::FinalPrefix)
            } else {
                Ok(())
            }
        }
        _ => Err(AdmissionRefusal::Operation),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding<'a>(
        attempt: u64,
        platform: &'a str,
        context: OperationContext<'a>,
    ) -> ResponseBinding<'a> {
        ResponseBinding {
            attempt,
            platform,
            context,
        }
    }

    fn materialize<'a>(closure_digest: &'a str, final_prefix: &'a str) -> OperationContext<'a> {
        OperationContext::Materialize {
            closure_digest,
            final_prefix,
        }
    }

    #[test]
    fn only_a_clean_exit_succeeds() {
        assert!(terminal_succeeded(NativeTerminal::Exited(0)));
        for terminal in [
            NativeTerminal::Exited(1),
            NativeTerminal::Exited(-1),
            NativeTerminal::Interrupted,
        ] {
            assert!(!terminal_succeeded(terminal), "{terminal:?}");
        }
    }

    #[test]
    fn a_response_is_admitted_only_for_its_exact_binding() {
        let contexts = [
            OperationContext::Resolve,
            OperationContext::ImportPixi,
            materialize("closure-a", "/prefix/a"),
        ];
        for expected in contexts {
            let request = binding(7, "linux-64", expected);
            assert_eq!(
                admit_response(NativeTerminal::Exited(0), request, request),
                Ok(())
            );
            for terminal in [NativeTerminal::Exited(1), NativeTerminal::Interrupted] {
                assert_eq!(
                    admit_response(terminal, request, request),
                    Err(AdmissionRefusal::NativeTerminal)
                );
            }
            assert_eq!(
                admit_response(
                    NativeTerminal::Exited(0),
                    request,
                    binding(8, "linux-64", expected)
                ),
                Err(AdmissionRefusal::Attempt)
            );
            assert_eq!(
                admit_response(
                    NativeTerminal::Exited(0),
                    request,
                    binding(7, "osx-arm64", expected)
                ),
                Err(AdmissionRefusal::Platform)
            );
            for actual in contexts {
                if core::mem::discriminant(&actual) != core::mem::discriminant(&expected) {
                    assert_eq!(
                        admit_response(
                            NativeTerminal::Exited(0),
                            request,
                            binding(7, "linux-64", actual)
                        ),
                        Err(AdmissionRefusal::Operation),
                        "{expected:?} answered as {actual:?}"
                    );
                }
            }
        }
        let request = binding(7, "linux-64", materialize("closure-a", "/prefix/a"));
        assert_eq!(
            admit_response(
                NativeTerminal::Exited(0),
                request,
                binding(7, "linux-64", materialize("closure-b", "/prefix/a"))
            ),
            Err(AdmissionRefusal::Closure)
        );
        assert_eq!(
            admit_response(
                NativeTerminal::Exited(0),
                request,
                binding(7, "linux-64", materialize("closure-a", "/prefix/b"))
            ),
            Err(AdmissionRefusal::FinalPrefix)
        );
    }
}
