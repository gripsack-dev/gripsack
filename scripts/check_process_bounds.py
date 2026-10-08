#!/usr/bin/env python3
"""Actual process/frame/IO/permission oracles with attributable seam mutants."""
from dataclasses import dataclass
from pathlib import Path
import ctypes
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
PROCESS = Path('crates/gripsack-process/src')
EXCHANGE = PROCESS / 'exchange.rs'
LIFECYCLE = PROCESS / 'lifecycle.rs'
FIXTURE = PROCESS / 'tests/fixtures/signal_denial.rs'
EXPECTED = (
    'deadline::tests::an_unfillable_wait_stops_later_admission',
    'deadline::tests::clock_observations_cannot_extend_or_revive_admission',
    'input::tests::rejected_append_preserves_the_admitted_request',
    'lifecycle::tests::reaped_leader_cannot_reauthorize_group_signals',
    'tests::expired_operation_rejects_spawn_without_overriding_input_failure',
    'tests::input_rejected_before_spawn',
    'tests::empty_input_is_closed_immediately',
    'tests::framing_and_input_eof',
    'tests::exact_line_limit_and_empty_lines',
    'tests::spawn_error_and_zero_budget',
    'tests::tail_is_exact_and_can_be_disabled',
    'tests::operation_and_exchange_deadlines_each_bound_cleanup',
    'tests::lifecycle::completed_cleanup_cannot_erase_an_expired_operation',
    'tests::lifecycle::no_response_is_a_normal_exit',
    'tests::lifecycle::callback_panic_kills_and_reaps_the_leader',
    'tests::lifecycle::exit_and_signal_status_are_not_routing_policy',
    'tests::lifecycle::inherited_pipes_are_killed_on_leader_exit_not_after_a_linger',
    'tests::lifecycle::silent_process_with_closed_pipes_still_has_a_deadline',
    'tests::native::spawn_failure_keeps_the_kernel_error_and_does_not_report_success',
    'tests::native::native_child_cannot_read_ungranted_environment_or_inherited_descriptor',
    'tests::native::retained_authority_survives_closed_standard_descriptors',
    'tests::native::raw_bytes_have_no_line_limit_but_keep_the_total_output_limit',
    'tests::native::selected_script_survives_source_replacement_and_stale_approval_refuses',
    'tests::pressure::response_still_sends_input_and_preserves_exit_failure',
    'tests::pressure::binary_stderr_tail_is_exact_across_pipe_chunks',
    'tests::pressure::simultaneous_input_and_output_pressure',
    'tests::pressure::huge_unterminated_line_is_stopped_incrementally',
    'tests::pressure::stdout_and_stderr_floods_are_cumulative',
    'tests::pressure::response_does_not_disable_any_output_bound',
    'tests::pressure::early_stdin_close_is_not_an_io_error',
    'tests::pressure::blocked_input_does_not_block_response_or_deadline',
)
PLATFORM_CASES = {
    'linux': (
        'tests::native::legacy_descriptor_sweep_hides_inherited_canary',
        'tests::native::legacy_memfd_flag_support_keeps_sealed_execution',
    ),
    'darwin': (
        'sys::macos::tests::unreaped_zombie_group_is_distinct_from_permission_denial',
        'sys::macos::tests::live_group_and_expired_inventory_are_not_dead_group_evidence',
    ),
}


@dataclass(frozen=True)
class Mutant:
    name: str
    source: Path
    before: str
    after: str
    case: str
    diagnostic: str


MUTANTS = (
    Mutant('serializer-input-observer', PROCESS / 'input.rs',
           '            self.bytes.capacity(),\n            bytes.len(),',
           '            self.bytes.capacity(),\n            0,',
           'input::tests::rejected_append_preserves_the_admitted_request',
           'serializer_excess_was_admitted'),
    Mutant('stdout-byte-observer', EXCHANGE,
           'self.stdout_budget.observe(n as u64)', 'self.stdout_budget.observe(0)',
           'tests::pressure::stdout_and_stderr_floods_are_cumulative',
           'stdout_total_was_not_enforced'),
    Mutant('stderr-byte-observer', EXCHANGE,
           'self.stderr_budget.observe(n as u64)', 'self.stderr_budget.observe(0)',
           'tests::pressure::stdout_and_stderr_floods_are_cumulative',
           'stderr_total_was_not_enforced'),
    Mutant('dropped-frame-admission', EXCHANGE,
           'match self.frame.observe(byte) {', 'match FrameAction::Append {',
           'tests::pressure::huge_unterminated_line_is_stopped_incrementally',
           'frame_limit_was_not_enforced'),
    Mutant('stderr-tail-effect', EXCHANGE,
           'self.tail.extend(&bytes[append.skip_bytes()..]);',
           'self.tail.extend(&bytes[..bytes.len() - append.skip_bytes()]);',
           'tests::pressure::binary_stderr_tail_is_exact_across_pipe_chunks',
           'stderr_tail_lost_exact_suffix'),
    Mutant('input-window-effect', EXCHANGE,
           'pipe.write(&self.input[chunk.range()])',
           'pipe.write(&self.input[..chunk.range().len()])',
           'tests::pressure::binary_stderr_tail_is_exact_across_pipe_chunks',
           'stderr_tail_lost_exact_suffix'),
    Mutant('cleanup-deadline-observer', LIFECYCLE,
           '.cleanup_decision(drained, self.deadline.remaining().is_some())',
           '.cleanup_decision(drained, true)',
           'tests::lifecycle::completed_cleanup_cannot_erase_an_expired_operation',
           'expired_cleanup_was_reported_successfully'),
    Mutant('reap-result-authority', LIFECYCLE,
           'Ok(Some(_)) => ReapObservation::Reaped,',
           'Ok(Some(_)) => ReapObservation::NotReady,',
           'lifecycle::tests::reaped_leader_cannot_reauthorize_group_signals',
           'reaped_leader_regained_signal_authority'),
    Mutant('signal-error-classification', PROCESS / 'sys.rs',
           'if error.raw_os_error() == Some(libc::ESRCH) {',
           'if matches!(error.raw_os_error(), Some(libc::ESRCH) | Some(libc::EPERM)) {',
           'permission', 'denied_group_signal_was_reported_successfully'),
)
# Linux records the FIRST denied signal in terminate() before finish(); the
# retained receipt is the clone made from the raw errno. Darwin's first
# permission denial instead surfaces inside Guard::finish, which records the
# original error directly, so the clone mutant is inert there: each platform
# gets the mutant that targets its actual errno-retention branch, observed by
# the same raw-errno fixture assertion (cleanup_errno_was_lost).
LINUX_MUTANTS = (
    Mutant('signal-errno-receipt', LIFECYCLE,
           'Some(code) => io::Error::from_raw_os_error(code),',
           'Some(_) => io::Error::new(error.kind(), error.to_string()),',
           'permission', 'cleanup_errno_was_lost'),
)
DARWIN_MUTANTS = (
    Mutant('darwin-cleanup-errno-receipt', LIFECYCLE,
           '                && e.kind() != io::ErrorKind::Interrupted\n'
           '            {\n'
           '                record(&mut error, e);\n'
           '            }',
           '                && e.kind() != io::ErrorKind::Interrupted\n'
           '            {\n'
           '                record(&mut error, io::Error::new(e.kind(), e.to_string()));\n'
           '            }',
           'permission', 'cleanup_errno_was_lost'),
    Mutant('darwin-live-group-observer', PROCESS / 'sys/macos.rs',
           'if info.pbi_status != libc::SZOMB {',
           'if info.pbi_status == libc::SZOMB {',
           'sys::macos::tests::live_group_and_expired_inventory_are_not_dead_group_evidence',
           'darwin_live_group_reported_dead'),
)


def run(root: Path, target: Path, command: list[str], *, timeout=600, preexec=None):
    print('RUNNER_COMMAND=' + ' '.join(command), flush=True)
    result = subprocess.run(command, cwd=root,
                            env={**os.environ, 'CARGO_TARGET_DIR': str(target)},
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            text=True, timeout=timeout, preexec_fn=preexec)
    print(result.stdout, flush=True)
    return result


def test(root: Path, target: Path, selection: str = ''):
    command = ['cargo', 'test', '--locked', '-p', 'gripsack-process', '--lib']
    if selection:
        # A single attributed case needs child diagnostics; the full-suite
        # positive run must keep capture ON so parallel child output cannot
        # split a libtest status line and hide a named oracle.
        command.extend([selection, '--', '--nocapture', '--exact'])
    return run(root, target, command)


def drop_kill_capability():
    # Child-only Linux bounding-set change. The runner keeps its credentials;
    # the fixture validates that exec did not restore CAP_KILL.
    library = ctypes.CDLL(None, use_errno=True)
    library.prctl.argtypes = [ctypes.c_int, ctypes.c_ulong, ctypes.c_ulong,
                             ctypes.c_ulong, ctypes.c_ulong]
    library.prctl.restype = ctypes.c_int
    pr_capbset_drop, cap_kill = 24, 5
    if library.prctl(pr_capbset_drop, cap_kill, 0, 0, 0) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))


def permission(root: Path, target: Path, *, live: bool):
    build = run(root, target, ['cargo', 'build', '--locked', '-p', 'gripsack-process',
                              '--example', 'signal_denial'])
    if build.returncode:
        raise SystemExit('FAIL: permission fixture build is not semantic evidence')
    binary = target / 'debug/examples/signal_denial'
    print('SIGNAL_FIXTURE_BINARY_SHA256=' + hashlib.sha256(binary.read_bytes()).hexdigest(), flush=True)
    command = [str(binary)]
    preexec = None
    if live:
        command.append('--live-credential-change')
    if sys.platform == 'darwin':
        command = ['/usr/bin/sudo', '-n', '--', *command]
    elif not live:
        preexec = drop_kill_capability
    return run(root, target, command, timeout=60, preexec=preexec)


def main():
    if sys.platform not in PLATFORM_CASES:
        raise SystemExit('FAIL: process qualification requires Linux or native macOS')
    if sys.platform == 'linux' and os.geteuid() != 0:
        raise SystemExit('FAIL: Linux permission qualification requires isolated container root')
    with tempfile.TemporaryDirectory(prefix='gripsack-process-bounds-') as temporary:
        temporary = Path(temporary)
        tree = temporary / 'source'
        tree.mkdir()
        for name in ('Cargo.toml', 'Cargo.lock'):
            shutil.copy2(ROOT / name, tree / name)
        for name in ('crates', 'fuzz', 'typescript', 'schema'):
            shutil.copytree(ROOT / name, tree / name, ignore=shutil.ignore_patterns(
                'target', 'node_modules', '.venv', '__pycache__',
            ))
        inputs = [Path('Cargo.toml'), Path('Cargo.lock')]
        for directory in (PROCESS, Path('crates/gripsack-policy/src/process_budget')):
            inputs.extend(path.relative_to(tree) for path in (tree / directory).rglob('*.rs'))
        inputs.extend((Path('crates/gripsack-policy/src/operation_budget.rs'),
                       Path('crates/gripsack-policy/src/process_budget.rs')))
        print('PROCESS_BOUNDS_INPUT_SHA256=' + json.dumps({str(path): hashlib.sha256(
            (tree / path).read_bytes()).hexdigest() for path in sorted(inputs)}, sort_keys=True), flush=True)
        target = temporary / 'target'
        positive = test(tree, target)
        expected = EXPECTED + PLATFORM_CASES[sys.platform]
        summary = re.search(r'test result: ok\. (\d+) passed; 0 failed; 0 ignored', positive.stdout)
        if positive.returncode or summary is None or int(summary[1]) < len(expected):
            raise SystemExit('FAIL: complete process oracle set did not pass')
        for case in expected:
            if f'test {case} ... ok' not in positive.stdout:
                raise SystemExit(f'FAIL: missing process oracle: {case}')
        example = tree / 'crates/gripsack-process/examples/signal_denial.rs'
        example.parent.mkdir(exist_ok=True)
        shutil.copy2(tree / FIXTURE, example)
        modes = (False, True) if sys.platform == 'linux' else (True,)
        for live in modes:
            result = permission(tree, target, live=live)
            if result.returncode or f'SIGNAL_DENIAL_PROPERTY=passed live={str(live).lower()}' not in result.stdout:
                raise SystemExit('FAIL: real signal permission boundary did not pass')
        print(f'PROCESS_BOUNDS_PROPERTIES={len(expected) + len(modes)}', flush=True)
        mutants = MUTANTS + (DARWIN_MUTANTS if sys.platform == 'darwin' else LINUX_MUTANTS)
        for mutant in mutants:
            source = tree / mutant.source
            original = source.read_text()
            if original.count(mutant.before) != 1:
                raise SystemExit(f'FAIL: {mutant.name} no longer uniquely matches')
            source.write_text(original.replace(mutant.before, mutant.after))
            try:
                if mutant.case == 'permission':
                    negative = permission(tree, target, live=sys.platform == 'darwin')
                    failed = negative.returncode != 0 and mutant.diagnostic in negative.stdout
                else:
                    negative = test(tree, target, mutant.case)
                    failed = (negative.returncode != 0
                              and f'test {mutant.case} ... FAILED' in negative.stdout
                              and mutant.diagnostic in negative.stdout
                              and 'test result: FAILED. 0 passed; 1 failed; 0 ignored' in negative.stdout)
            finally:
                source.write_text(original)
            if not failed or 'could not compile' in negative.stdout:
                raise SystemExit(f'FAIL: {mutant.name} missed its named process effect oracle')
            print(f'calibration: {mutant.name} rejected by {mutant.case}', flush=True)
        print(f'PROCESS_BOUNDS_MUTANTS={len(mutants)}', flush=True)
        print('Process bounds gate: OK', flush=True)


if __name__ == '__main__':
    try:
        main()
    except subprocess.TimeoutExpired as error:
        raise SystemExit('FAIL: timed-out process verification is not semantic calibration') from error
