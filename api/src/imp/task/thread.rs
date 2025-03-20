use core::ffi::c_char;

use alloc::vec::Vec;
use axerrno::{LinuxError, LinuxResult};
use axtask::{TaskExtRef, current, yield_now};
use macro_rules_attribute::apply;
use num_enum::TryFromPrimitive;
use starry_core::task::{CloneFlags, WaitError, exec, wait_pid};

use crate::{
    ptr::{UserConstPtr, UserPtr, nullable},
    syscall_instrument,
};

/// ARCH_PRCTL codes
///
/// It is only avaliable on x86_64, and is not convenient
/// to generate automatically via c_to_rust binding.
#[derive(Debug, Eq, PartialEq, TryFromPrimitive)]
#[repr(i32)]
enum ArchPrctlCode {
    /// Set the GS segment base
    SetGs = 0x1001,
    /// Set the FS segment base
    SetFs = 0x1002,
    /// Get the FS segment base
    GetFs = 0x1003,
    /// Get the GS segment base
    GetGs = 0x1004,
    /// The setting of the flag manipulated by ARCH_SET_CPUID
    GetCpuid = 0x1011,
    /// Enable (addr != 0) or disable (addr == 0) the cpuid instruction for the calling thread.
    SetCpuid = 0x1012,
}

#[apply(syscall_instrument)]
pub fn sys_getpid() -> LinuxResult<isize> {
    Ok(axtask::current().task_ext().proc_id as _)
}

#[apply(syscall_instrument)]
pub fn sys_getppid() -> LinuxResult<isize> {
    Ok(axtask::current().task_ext().get_parent() as _)
}

pub fn sys_exit(status: i32) -> ! {
    let curr = current();
    let clear_child_tid = curr.task_ext().clear_child_tid() as *mut i32;
    if !clear_child_tid.is_null() {
        // TODO: check whether the address is valid
        unsafe {
            // TODO: Encapsulate all operations that access user-mode memory into a unified function
            *(clear_child_tid) = 0;
        }
        // TODO: wake up threads, which are blocked by futex, and waiting for the address pointed by clear_child_tid
    }
    axtask::exit(status);
}

pub fn sys_exit_group(status: i32) -> ! {
    warn!("Temporarily replace sys_exit_group with sys_exit");
    axtask::exit(status);
}

/// To set the clear_child_tid field in the task extended data.
///
/// The set_tid_address() always succeeds
#[apply(syscall_instrument)]
pub fn sys_set_tid_address(tid_ptd: UserConstPtr<i32>) -> LinuxResult<isize> {
    let curr = current();
    curr.task_ext()
        .set_clear_child_tid(tid_ptd.address().as_ptr() as _);
    Ok(curr.id().as_u64() as isize)
}

#[cfg(target_arch = "x86_64")]
#[apply(syscall_instrument)]
pub fn sys_arch_prctl(code: i32, addr: UserPtr<u64>) -> LinuxResult<isize> {
    use axerrno::LinuxError;
    match ArchPrctlCode::try_from(code).map_err(|_| LinuxError::EINVAL)? {
        // According to Linux implementation, SetFs & SetGs does not return
        // error at all
        ArchPrctlCode::SetFs => {
            unsafe {
                axhal::arch::write_thread_pointer(addr.address().as_usize());
            }
            Ok(0)
        }
        ArchPrctlCode::SetGs => {
            unsafe {
                x86::msr::wrmsr(x86::msr::IA32_KERNEL_GSBASE, addr.address().as_usize() as _);
            }
            Ok(0)
        }
        ArchPrctlCode::GetFs => {
            *addr.get_as_mut()? = axhal::arch::read_thread_pointer() as u64;
            Ok(0)
        }

        ArchPrctlCode::GetGs => {
            *addr.get_as_mut()? = unsafe { x86::msr::rdmsr(x86::msr::IA32_KERNEL_GSBASE) };
            Ok(0)
        }
        ArchPrctlCode::GetCpuid => Ok(0),
        ArchPrctlCode::SetCpuid => Err(LinuxError::ENODEV),
    }
}

#[apply(syscall_instrument)]
pub fn sys_clone(
    flags: usize,
    user_stack: usize,
    ptid: usize,
    arg3: usize,
    arg4: usize,
) -> LinuxResult<isize> {
    let flags = CloneFlags::from_bits((flags & !0x3f) as u32).unwrap();
    let tls = arg3;
    let ctid = arg4;

    let stack = if user_stack == 0 {
        None
    } else {
        Some(user_stack)
    };

    let curr_task = current();

    if let Ok(new_task_id) = curr_task
        .task_ext()
        .clone_task(flags, stack, ptid, tls, ctid)
    {
        Ok(new_task_id as isize)
    } else {
        Err(LinuxError::ENOMEM)
    }
}

bitflags::bitflags! {
    pub struct WaitFlags: u32 {
        /// 不挂起当前进程，直接返回
        const WNOHANG = 1 << 0;
        /// 报告已执行结束的用户进程的状态
        const WIMTRACED = 1 << 1;
        /// 报告还未结束的用户进程的状态
        const WCONTINUED = 1 << 3;
        /// Wait for any child
        const WALL = 1 << 30;
        /// Wait for cloned process
        const WCLONE = 1 << 31;
    }
}

#[apply(syscall_instrument)]
pub fn sys_wait4(pid: i32, exit_code: UserPtr<i32>, option: u32) -> LinuxResult<isize> {
    let option_flag = WaitFlags::from_bits(option).unwrap();
    loop {
        let answer = wait_pid(pid);
        match answer {
            Ok((pid, code)) => {
                if let Some(exit_code) = nullable!(exit_code.get_as_mut())? {
                    *exit_code = code << 8;
                }
                return Ok(pid as isize);
            }
            Err(status) => match status {
                WaitError::NotFound => {
                    return Err(LinuxError::ECHILD);
                }
                WaitError::Running => {
                    if option_flag.contains(WaitFlags::WNOHANG) {
                        return Ok(0);
                    } else {
                        yield_now();
                    }
                }
            },
        }
    }
}

#[apply(syscall_instrument)]
pub fn sys_execve(
    path: UserConstPtr<c_char>,
    argv: UserConstPtr<UserConstPtr<c_char>>,
    envp: UserConstPtr<UserConstPtr<c_char>>,
) -> LinuxResult<isize> {
    let path_str = path.get_as_str()?;

    let args = argv
        .get_as_null_terminated()?
        .iter()
        .map(|arg| arg.get_as_str().map(Into::into))
        .collect::<Result<Vec<_>, _>>()?;
    let envs = envp
        .get_as_null_terminated()?
        .iter()
        .map(|env| env.get_as_str().map(Into::into))
        .collect::<Result<Vec<_>, _>>()?;

    info!(
        "execve: path: {:?}, args: {:?}, envs: {:?}",
        path_str, args, envs
    );

    if let Err(e) = exec(path_str, &args, &envs) {
        error!("Failed to exec: {:?}", e);
        return Err::<isize, _>(LinuxError::ENOSYS);
    }

    unreachable!("execve should never return");
}
