use core::{mem, time::Duration};

use arceos_posix_api::ctypes::timespec;
use axerrno::{LinuxError, LinuxResult};
use axprocess::{Pid, Process, ProcessGroup, Thread};
use linux_raw_sys::general::{
    kernel_sigaction, siginfo, SIG_BLOCK, SIG_SETMASK, SIG_UNBLOCK, SI_TKILL, SI_USER
};
use starry_core::task::{get_process, get_process_group, get_thread, processes, ProcessData, ThreadData};

use crate::ptr::{PtrWrapper, UserConstPtr, UserPtr};

use axhal::{
    arch::TrapFrame,
    trap::{POST_TRAP, register_trap_handler},
};
use axsignal::{SignalInfo, SignalOSAction, SignalSet, Signo};
use axtask::{TaskExtRef, current};

use super::do_exit;

fn check_signals(tf: &mut TrapFrame, restore_blocked: Option<SignalSet>) -> bool {
    let Some((sig, os_action)) = current()
        .task_ext()
        .thread_data()
        .signal
        .check_signals(tf, restore_blocked)
    else {
        return false;
    };

    let signo = sig.signo();
    match os_action {
        SignalOSAction::Terminate => {
            do_exit(128 + signo as i32, true);
        }
        SignalOSAction::CoreDump => {
            // TODO: implement core dump
            do_exit(128 + signo as i32, true);
        }
        SignalOSAction::Stop => {
            // TODO: implement stop
            do_exit(1, true);
        }
        SignalOSAction::Continue => {
            // TODO: implement continue
        }
        SignalOSAction::Handler => {
            // do nothing
        }
    }
    true
}

#[register_trap_handler(POST_TRAP)]
fn post_trap_callback(tf: &mut TrapFrame, from_user: bool) {
    if !from_user {
        return;
    }

    check_signals(tf, None);
}

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

    current()
        .task_ext()
        .thread_data()
        .signal
        .with_blocked_mut::<LinuxResult<_>>(|blocked| {
            if let Some(oldset) = oldset.nullable(UserPtr::get)? {
                unsafe { *oldset = *blocked };
            }

            if let Some(set) = set.nullable(UserConstPtr::get)? {
                let set = unsafe { *set };
                match how as u32 {
                    SIG_BLOCK => *blocked |= set,
                    SIG_UNBLOCK => *blocked &= !set,
                    SIG_SETMASK => *blocked = set,
                    _ => return Err(LinuxError::EINVAL),
                }
            }
            Ok(())
        })?;

    Ok(0)
}

pub fn sys_rt_sigaction(
    signo: i32,
    act: UserConstPtr<kernel_sigaction>,
    oldact: UserPtr<kernel_sigaction>,
    sigsetsize: usize,
) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;

    let signo = Signo::from_repr(signo as u8).ok_or(LinuxError::EINVAL)?;
    if matches!(signo, Signo::SIGKILL | Signo::SIGSTOP) {
        return Err(LinuxError::EINVAL);
    }

    let curr = current();
    curr.task_ext()
        .process_data()
        .signal
        .with_action_mut::<LinuxResult<_>>(signo, |action| {
            if let Some(oldact) = oldact.nullable(UserPtr::get)? {
                action.to_ctype(unsafe { &mut *oldact });
            }
            if let Some(act) = act.nullable(UserConstPtr::get)? {
                *action = unsafe { (*act).try_into()? };
            }
            Ok(())
        })?;

    Ok(0)
}

pub fn sys_rt_sigpending(set: UserPtr<SignalSet>, sigsetsize: usize) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;
    unsafe {
        *set.get()? = current().task_ext().thread_data().signal.pending();
    }
    Ok(0)
}

pub fn send_signal_thread(thr: &Thread, sig: SignalInfo) -> LinuxResult<()> {
    info!("Send signal {:?} to thread {}", sig.signo(), thr.tid());
    let Some(thr) = thr.data::<ThreadData>() else {
        return Err(LinuxError::EPERM);
    };
    thr.signal.send_signal(sig);
    Ok(())
}
pub fn send_signal_process(proc: &Process, sig: SignalInfo) -> LinuxResult<()> {
    info!("Send signal {:?} to process {}", sig.signo(), proc.pid());
    let Some(proc) = proc.data::<ProcessData>() else {
        return Err(LinuxError::EPERM);
    };
    proc.signal.send_signal(sig);
    Ok(())
}
pub fn send_signal_process_group(pg: &ProcessGroup, sig: SignalInfo) -> usize {
    info!(
        "Send signal {:?} to process group {}",
        sig.signo(),
        pg.pgid()
    );
    let mut count = 0;
    for proc in pg.processes() {
        count += send_signal_process(&proc, sig.clone()).is_ok() as usize;
    }
    count
}

fn make_siginfo(signo: u32, code: u32) -> LinuxResult<Option<SignalInfo>> {
    if signo == 0 {
        return Ok(None);
    }
    let signo = Signo::from_repr(signo as u8).ok_or(LinuxError::EINVAL)?;
    Ok(Some(SignalInfo::new(signo, code)))
}

pub fn sys_kill(pid: i32, sig: u32) -> LinuxResult<isize> {
    let Some(sig) = make_siginfo(sig, SI_USER)? else {
        // TODO: should also check permissions
        return Ok(0);
    };

    let curr = current();
    let mut result = 0usize;
    match pid {
        1.. => {
            let proc = get_process(pid as Pid)?;
            send_signal_process(&proc, sig)?;
            result += 1;
        }
        0 => {
            let pg = curr.task_ext().thread.process().group();
            result += send_signal_process_group(&pg, sig);
        }
        -1 => {
            for proc in processes() {
                if proc.is_init() {
                    // init process
                    continue;
                }
                send_signal_process(&proc, sig.clone())?;
                result += 1;
            }
        }
        ..-1 => {
            let pg = get_process_group((-pid) as Pid)?;
            result += send_signal_process_group(&pg, sig);
        }
    }

    Ok(result as isize)
}

pub fn sys_tkill(tid: Pid, sig: u32) -> LinuxResult<isize> {
    let Some(sig) = make_siginfo(sig, SI_TKILL as u32)? else {
        // TODO: should also check permissions
        return Ok(0);
    };

    let thr = get_thread(tid)?;
    send_signal_thread(&thr, sig)?;
    Ok(0)
}

pub fn sys_tgkill(tgid: Pid, tid: Pid, sig: u32) -> LinuxResult<isize> {
    let Some(sig) = make_siginfo(sig, SI_TKILL as u32)? else {
        // TODO: should also check permissions
        return Ok(0);
    };

    let thr = get_thread(tid)?;
    if thr.process().pid() != tgid {
        return Err(LinuxError::ESRCH);
    }
    send_signal_thread(&thr, sig)?;
    Ok(0)
}

pub fn sys_rt_sigreturn(tf: &mut TrapFrame) -> LinuxResult<isize> {
    let curr = current();
    curr.task_ext().thread_data().signal.restore(tf);
    Ok(tf.retval() as isize)
}

pub fn sys_rt_sigtimedwait(
    set: UserConstPtr<SignalSet>,
    info: UserPtr<siginfo>,
    timeout: UserConstPtr<timespec>,
    sigsetsize: usize,
) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;

    let set = unsafe { *set.get()? };
    let timeout: Option<Duration> = timeout
        .nullable(UserConstPtr::get)?
        .map(|ts| unsafe { *ts }.into());

    let Some(sig) = current()
        .task_ext()
        .thread_data()
        .signal
        .wait_timeout(set, timeout)
    else {
        return Err(LinuxError::EAGAIN);
    };

    if let Some(info) = info.nullable(UserPtr::get)? {
        sig.to_ctype(unsafe { &mut *info });
    }

    Ok(0)
}

pub fn sys_rt_sigsuspend(
    tf: &mut TrapFrame,
    set: UserPtr<SignalSet>,
    sigsetsize: usize,
) -> LinuxResult<isize> {
    check_sigset_size(sigsetsize)?;

    let curr = current();
    let thr_data = curr.task_ext().thread_data();
    let mut set = unsafe { *set.get()? };

    set.remove(Signo::SIGKILL);
    set.remove(Signo::SIGSTOP);

    let old_blocked = thr_data
        .signal
        .with_blocked_mut(|blocked| mem::replace(blocked, set));

    tf.set_retval((-LinuxError::EINTR.code() as isize) as usize);

    loop {
        if check_signals(tf, Some(old_blocked)) {
            break;
        }
        curr.task_ext().process_data().signal.wait_signal();
    }

    Ok(0)
}
