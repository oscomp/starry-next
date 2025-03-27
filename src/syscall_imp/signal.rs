use axerrno::{LinuxError, LinuxResult};
use axhal::arch::TrapFrame;
use axtask::{TaskExtRef, current};

use crate::{
    ptr::{PtrWrapper, UserConstPtr, UserPtr},
    signal::{
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
            warn!("killing specified pid is not implemented");
            return Err(LinuxError::ENOSYS);
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
