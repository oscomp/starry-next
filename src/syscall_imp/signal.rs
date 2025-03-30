use core::time::Duration;

use arceos_posix_api as api;
use axerrno::{LinuxError, LinuxResult};
use axhal::arch::TrapFrame;
use axtask::{TaskExtRef, current};

use crate::{
    ptr::{PtrWrapper, UserConstPtr, UserPtr},
    signal::{
        SIGKILL, SIGSTOP,
        ctypes::{SignalAction, SignalInfo, SignalSet, k_sigaction},
        sigreturn,
    },
};

fn check_sigset_size(size: usize) -> LinuxResult<()> {
    if size != size_of::<SignalSet>() {
        return Err(LinuxError::EINVAL);
    }
    Ok(())
}

pub fn sys_rt_sigprocmask(
    how: i32,
    set: UserConstPtr<SignalSet>,
    oldset: UserPtr<SignalSet>,
    sigsetsize: usize,
) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;

    let curr = current();
    let mut sigman = curr.task_ext().signal.lock();

    if let Some(oldset) = oldset.nullable(UserPtr::get)? {
        // SAFETY: oldset is a valid pointer
        unsafe {
            oldset.write(sigman.blocked);
        }
    }

    if let Some(set) = set.nullable(UserConstPtr::get)? {
        // SAFETY: set is a valid pointer
        let set: SignalSet = unsafe { set.read() };
        match how {
            // SIG_BLOCK
            0 => sigman.blocked.add_from(&set),
            // SIG_UNBLOCK
            1 => sigman.blocked.remove_from(&set),
            // SIG_SETMASK
            2 => sigman.blocked = set,
            _ => return Err(LinuxError::EINVAL),
        }
    }

    Ok(0)
}

pub fn sys_rt_sigaction(
    signum: i32,
    act: UserConstPtr<k_sigaction>,
    oldact: UserPtr<k_sigaction>,
    sigsetsize: usize,
) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;

    let signum = signum as u32;
    if !(1..32).contains(&signum) {
        return Err(LinuxError::EINVAL);
    }
    if signum == crate::signal::SIGKILL || signum == crate::signal::SIGSTOP {
        return Err(LinuxError::EINVAL);
    }

    let curr = current();
    let mut sigman = curr.task_ext().signal.lock();

    if let Some(oldact) = oldact.nullable(UserPtr::get)? {
        // SAFETY: oldact is a valid pointer
        sigman.action(signum).to_ctype(unsafe { &mut *oldact });
    }

    if let Some(act) = act.nullable(UserConstPtr::get)? {
        // SAFETY: act is a valid pointer
        let action: SignalAction = unsafe { act.read() }.try_into()?;
        sigman.set_action(signum, action);
    }

    Ok(0)
}

pub fn sys_rt_sigpending(set: UserPtr<SignalSet>, sigsetsize: usize) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;

    let curr = current();
    let sigman = curr.task_ext().signal.lock();

    let set = set.get()?;
    // SAFETY: set is a valid pointer
    unsafe { set.write(sigman.pending) };

    Ok(0)
}

pub fn sys_rt_sigreturn(tf: &mut TrapFrame) -> LinuxResult<isize> {
    sigreturn(tf);
    Ok(tf.retval() as _)
}

pub fn sys_rt_sigtimedwait(
    set: UserConstPtr<SignalSet>,
    info: UserPtr<SignalInfo>,
    timeout: UserConstPtr<api::ctypes::timespec>,
    sigsetsize: usize,
) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;

    let set = set.get()?;
    // SAFETY: set is a valid pointer
    let mut set = SignalSet::from(unsafe { set.read() });

    let timeout = timeout.nullable(UserConstPtr::get)?.map(|spec| {
        // SAFETY: spec is a valid pointer
        let spec = unsafe { spec.read() };
        Duration::new(spec.tv_sec as u64, spec.tv_nsec as u32)
    });

    let curr = current();
    let mut sigman = curr.task_ext().signal.lock();

    // Non-blocked signals cannot be waited
    set.remove_from(&!sigman.blocked);

    if let Some(siginfo) = sigman.dequeue_signal_in(&set) {
        if let Some(info) = info.nullable(UserPtr::get)? {
            // SAFETY: info is a valid pointer
            unsafe { info.write(siginfo) };
        }
        return Ok(0);
    }

    sigman.waiting = set;
    let wq = sigman.wq.clone();
    drop(sigman);

    if let Some(timeout) = timeout {
        wq.wait_timeout(timeout);
    } else {
        wq.wait();
    }

    let mut sigman = curr.task_ext().signal.lock();
    if let Some(siginfo) = sigman.dequeue_signal_in(&set) {
        if let Some(info) = info.nullable(UserPtr::get)? {
            // SAFETY: info is a valid pointer
            unsafe { info.write(siginfo) };
        }
        return Ok(0);
    }

    // TODO: EINTR

    Err(LinuxError::EAGAIN)
}

pub fn sys_rt_sigsuspend(
    tf: &mut TrapFrame,
    set: UserPtr<SignalSet>,
    sigsetsize: usize,
) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;

    let curr = current();
    let set = set.get()?;
    // SAFETY: set is a valid pointer
    let mut set = unsafe { *set };

    set.remove(SIGKILL);
    set.remove(SIGSTOP);

    let mut sigman = curr.task_ext().signal.lock();
    let old_blocked = sigman.blocked;

    sigman.blocked = set;
    sigman.waiting = !set;
    drop(sigman);

    // TODO: document this
    #[cfg(any(
        target_arch = "riscv32",
        target_arch = "riscv64",
        target_arch = "loongarch64"
    ))]
    tf.set_ip(tf.ip() + 4);
    tf.set_retval((-LinuxError::EINTR.code() as isize) as usize);

    'wait: loop {
        let mut sigman = curr.task_ext().signal.lock();
        while let Some(sig) = sigman.dequeue_signal() {
            if sigman.run_action(tf, sig) {
                sigman.blocked = old_blocked;
                sigman.prevent_signal_handling = true;
                break 'wait;
            }
        }
        let wq = sigman.wq.clone();
        drop(sigman);
        wq.wait();
    }
    Ok(0)
}

pub fn sys_kill(pid: i32, sig: i32) -> LinuxResult<isize> {
    if !(1..32).contains(&sig) {
        return Err(LinuxError::EINVAL);
    }
    if sig == 0 {
        // TODO: should also check permissions
        return Ok(0);
    }

    let sig = SignalInfo::new(sig as u32, SignalInfo::SI_USER);

    let curr = current();
    let mut result = 0;
    match pid {
        1.. => {
            warn!("killing specified pid is only supported for child processes and self");
            if curr.id().as_u64() == pid as u64 {
                result += curr.task_ext().signal.lock().send_signal(sig) as isize;
            } else {
                let children = curr.task_ext().children.lock();
                let child = children.iter().find(|it| it.id().as_u64() == pid as u64);
                if let Some(child) = child {
                    result += child.task_ext().signal.lock().send_signal(sig) as isize;
                } else {
                    return Err(LinuxError::ESRCH);
                }
            }
        }
        0 => {
            // TODO: this should send signal to the current process group
            result += curr.task_ext().signal.lock().send_signal(sig) as isize;
        }
        -1 => {
            // TODO: this should "send signal to every process for which the
            // calling process has permission to send signals, except for
            // process 1"
            let mut guard = curr.task_ext().signal.lock();
            for child in curr.task_ext().children.lock().iter() {
                result += child.task_ext().signal.lock().send_signal(sig.clone()) as isize;
            }
            result += guard.send_signal(sig) as isize;
        }
        ..-1 => {
            warn!("killing specified process group is not implemented");
            return Err(LinuxError::ENOSYS);
        }
    }

    Ok(result)
}
