use alloc::{sync::Arc, vec::Vec};
use axerrno::{LinuxError, LinuxResult};
use axprocess::{Pid, Process};
use axtask::{TaskExtRef, current};
use bitflags::bitflags;
use macro_rules_attribute::apply;

use crate::{
    ptr::{PtrWrapper, UserPtr},
    syscall_instrument,
};

bitflags! {
    #[derive(Debug)]
    struct WaitOptions: u32 {
        const WNOHANG = 1 << 0;

        const WUNTRACED = 1 << 1;
        const WEXITED = 1 << 2;
        const WCONTINUED = 1 << 3;

        const WNOWAIT = 1 << 24;

        const WNOTHREAD = 1 << 29;
        const WALL = 1 << 30;
        const WCLONE = 1 << 31;
    }
}

#[derive(Debug, Clone, Copy)]
enum WaitPid {
    /// Wait for any child process
    Any,
    /// Wait for the child whose process ID is equal to the value.
    Pid(Pid),
    /// Wait for any child process whose process group ID is equal to the value.
    Pgid(Pid),
}

impl WaitPid {
    fn apply(&self, child: &Arc<Process>) -> bool {
        match self {
            WaitPid::Any => true,
            WaitPid::Pid(pid) => child.pid() == *pid,
            WaitPid::Pgid(pgid) => child.group().pgid() == *pgid,
        }
    }
}

#[apply(syscall_instrument)]
pub fn sys_waitpid(pid: i32, exit_code_ptr: UserPtr<i32>, options: u32) -> LinuxResult<isize> {
    let options = WaitOptions::from_bits_truncate(options);
    info!("sys_waitpid <= pid: {:?}, options: {:?}", pid, options);

    let curr = current();
    let process = curr.task_ext().thread.process();

    let pid = if pid == -1 {
        WaitPid::Any
    } else if pid == 0 {
        WaitPid::Pgid(process.group().pgid())
    } else if pid > 0 {
        WaitPid::Pid(pid as _)
    } else {
        WaitPid::Pgid(-pid as _)
    };

    let children = process
        .children()
        .into_iter()
        .filter(|child| pid.apply(child))
        .collect::<Vec<_>>();
    if children.is_empty() {
        return Err(LinuxError::ECHILD);
    }

    let exit_code = exit_code_ptr.nullable(UserPtr::get)?;
    loop {
        if let Some(child) = children.iter().find(|child| child.is_zombie()) {
            if !options.contains(WaitOptions::WNOWAIT) {
                child.free();
            }
            if let Some(exit_code) = exit_code {
                unsafe { exit_code.write(child.exit_code()) };
            }
            return Ok(child.pid() as _);
        } else if options.contains(WaitOptions::WNOHANG) {
            return Ok(0);
        } else {
            // TODO: process wait queue
            crate::sys_sched_yield()?;
        }
    }
}
