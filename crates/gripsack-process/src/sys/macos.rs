//! Darwin filters zombies out of killpg's candidate set, so EPERM alone does
//! not distinguish a dead group from a live group that we cannot signal.
//! See xnu/bsd/kern/kern_sig.c::killpg1 and proc_info.c::proc_listpids.
use std::io;
use std::time::Instant;

#[derive(PartialEq, Eq)]
struct ZombieIdentity {
    pid: libc::pid_t,
    started_seconds: u64,
    started_microseconds: u64,
}

enum MemberObservation {
    Gone,
    Zombie(ZombieIdentity),
    Live,
}

pub(super) fn group_is_zombie_only(group: libc::pid_t, deadline: Instant) -> io::Result<bool> {
    remaining(deadline)?;
    // SAFETY: a null buffer requests the documented PID-count allocation hint.
    let hint = unsafe { libc::proc_listpgrppids(group, std::ptr::null_mut(), 0) };
    if hint < 0 {
        return Err(io::Error::last_os_error());
    }
    if hint == 0 {
        return Ok(false);
    }
    let mut pids = Vec::new();
    pids.try_reserve_exact(hint as usize)
        .map_err(io::Error::other)?;
    read_group(group, &mut pids, deadline)?;
    if !pids.contains(&group) {
        return Ok(false);
    }
    pids.sort_unstable();
    let mut zombies = Vec::new();
    zombies
        .try_reserve_exact(pids.len())
        .map_err(io::Error::other)?;
    for &pid in &pids {
        match observe_member(group, pid, deadline)? {
            MemberObservation::Zombie(identity) => zombies.push(identity),
            MemberObservation::Gone => {}
            MemberObservation::Live => return Ok(false),
        }
    }
    // A process could have forked between the first membership snapshot and
    // its zombie observation. Re-enumerate; only those already-dead identities
    // may remain. Birth times distinguish reuse of a reaped descendant PID.
    // The unreaped leader pins the group identity throughout this operation.
    read_group(group, &mut pids, deadline)?;
    if !pids.contains(&group) {
        return Ok(false);
    }
    for &pid in &pids {
        let Ok(index) = zombies.binary_search_by_key(&pid, |identity| identity.pid) else {
            return Ok(false);
        };
        match observe_member(group, pid, deadline)? {
            MemberObservation::Zombie(identity) if identity == zombies[index] => {}
            MemberObservation::Gone => {}
            _ => return Ok(false),
        }
    }
    remaining(deadline)?;
    Ok(true)
}

fn read_group(
    group: libc::pid_t,
    pids: &mut Vec<libc::pid_t>,
    deadline: Instant,
) -> io::Result<()> {
    remaining(deadline)?;
    let bytes = pids
        .capacity()
        .checked_mul(size_of::<libc::pid_t>())
        .and_then(|bytes| libc::c_int::try_from(bytes).ok())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "process inventory size overflow",
            )
        })?;
    // SAFETY: capacity provides writable storage for `bytes`. libproc returns
    // a count of initialized PIDs, not bytes. No references into the Vec exist.
    let count = unsafe { libc::proc_listpgrppids(group, pids.as_mut_ptr().cast(), bytes) };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    let count = count as usize;
    if count >= pids.capacity() {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "process group inventory may be truncated",
        ));
    }
    // SAFETY: the successful syscall initialized precisely these PID entries;
    // the checked count is strictly within the reserved allocation.
    unsafe { pids.set_len(count) };
    Ok(())
}

fn observe_member(
    group: libc::pid_t,
    pid: libc::pid_t,
    deadline: Instant,
) -> io::Result<MemberObservation> {
    remaining(deadline)?;
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: the output buffer has the exact documented ABI size. Argument 1
    // includes zombie lookup; info is read only after a complete result.
    let count = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            1,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if count <= 0 {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(MemberObservation::Gone)
        } else {
            Err(error)
        };
    }
    if count != size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incomplete process identity",
        ));
    }
    // SAFETY: the successful full-sized query initialized all struct fields.
    let info = unsafe { info.assume_init() };
    if info.pbi_pid != pid as u32 || info.pbi_pgid != group as u32 {
        return Ok(MemberObservation::Gone);
    }
    if info.pbi_status != libc::SZOMB {
        return Ok(MemberObservation::Live);
    }
    Ok(MemberObservation::Zombie(ZombieIdentity {
        pid,
        started_seconds: info.pbi_start_tvsec,
        started_microseconds: info.pbi_start_tvusec,
    }))
}

fn remaining(deadline: Instant) -> io::Result<()> {
    if Instant::now() < deadline {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "process inventory deadline expired",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::Guard;
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Command, Stdio};
    use std::time::Duration;

    fn leader(body: &str) -> (libc::pid_t, Guard, Instant) {
        let child = Command::new("/bin/sh")
            .args(["-c", body])
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        let pid = child.id() as libc::pid_t;
        let (guard, _pipes) = Guard::new(child, end);
        (pid, guard, end)
    }

    #[test]
    fn unreaped_zombie_group_is_distinct_from_permission_denial() {
        let (pid, mut guard, end) = leader(":");
        while !guard.observe().unwrap() {
            assert!(Instant::now() < end, "child did not exit");
            std::thread::yield_now();
        }
        assert!(
            group_is_zombie_only(pid, end).unwrap(),
            "darwin_zombie_group_not_admitted",
        );
        let (status, error) = guard.finish(|_| Ok(true));
        assert!(error.is_none(), "{error:?}");
        assert_eq!(status.unwrap().code(), Some(0));
    }

    #[test]
    fn live_group_and_expired_inventory_are_not_dead_group_evidence() {
        let (pid, mut guard, end) = leader("exec sleep 60");
        assert!(
            !group_is_zombie_only(pid, end).unwrap(),
            "darwin_live_group_reported_dead",
        );
        assert_eq!(
            group_is_zombie_only(pid, Instant::now())
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut,
        );
        let (status, error) = guard.finish(|_| Ok(true));
        assert!(error.is_none(), "{error:?}");
        assert_eq!(status.unwrap().signal(), Some(libc::SIGKILL));
    }
}
