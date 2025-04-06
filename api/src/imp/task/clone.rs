use alloc::sync::Arc;
use arceos_posix_api::FD_TABLE;
use axerrno::{LinuxError, LinuxResult};
use axfs::{CURRENT_DIR, CURRENT_DIR_PATH};
use axhal::arch::{TrapFrame, UspaceContext};
use axprocess::{Pid, ProcessBuilder, ThreadBuilder};
use axsync::Mutex;
use axtask::{TaskExtRef, current};
use bitflags::bitflags;
use macro_rules_attribute::apply;
use starry_core::{
    mm::copy_from_kernel,
    task::{ProcessData, TaskExt, ThreadData, add_thread_to_table, new_user_task},
};

use crate::syscall_instrument;

bitflags! {
    /// 用于 sys_clone 的选项
    #[derive(Debug, Clone, Copy, Default)]
    struct CloneFlags: u32 {
        /// .
        const CLONE_NEWTIME = 1 << 7;
        /// 共享地址空间
        const CLONE_VM = 1 << 8;
        /// 共享文件系统新信息
        const CLONE_FS = 1 << 9;
        /// 共享文件描述符(fd)表
        const CLONE_FILES = 1 << 10;
        /// 共享信号处理函数
        const CLONE_SIGHAND = 1 << 11;
        /// 创建指向子任务的fd，用于 sys_pidfd_open
        const CLONE_PIDFD = 1 << 12;
        /// 用于 sys_ptrace
        const CLONE_PTRACE = 1 << 13;
        /// 指定父任务创建后立即阻塞，直到子任务退出才继续
        const CLONE_VFORK = 1 << 14;
        /// 指定子任务的 ppid 为当前任务的 ppid，相当于创建“兄弟”而不是“子女”
        const CLONE_PARENT = 1 << 15;
        /// 作为一个“线程”被创建。具体来说，它同 CLONE_PARENT 一样设置 ppid，且不可被 wait
        const CLONE_THREAD = 1 << 16;
        /// 子任务使用新的命名空间。目前还未用到
        const CLONE_NEWNS = 1 << 17;
        /// 子任务共享同一组信号量。用于 sys_semop
        const CLONE_SYSVSEM = 1 << 18;
        /// 要求设置 tls
        const CLONE_SETTLS = 1 << 19;
        /// 要求在父任务的一个地址写入子任务的 tid
        const CLONE_PARENT_SETTID = 1 << 20;
        /// 要求将子任务的一个地址清零。这个地址会被记录下来，当子任务退出时会触发此处的 futex
        const CLONE_CHILD_CLEARTID = 1 << 21;
        /// 历史遗留的 flag，现在按 linux 要求应忽略
        const CLONE_DETACHED = 1 << 22;
        /// 与 sys_ptrace 相关，目前未用到
        const CLONE_UNTRACED = 1 << 23;
        /// 要求在子任务的一个地址写入子任务的 tid
        const CLONE_CHILD_SETTID = 1 << 24;
        /// New pid namespace.
        const CLONE_NEWPID = 1 << 29;
    }
}

#[apply(syscall_instrument)]
pub fn sys_clone(
    flags: u32,
    stack: usize,
    _ptid: usize,
    _tls: usize,
    _ctid: usize,
) -> LinuxResult<isize> {
    const FLAG_MASK: u32 = 0xff;
    let _exit_signal = flags & FLAG_MASK;
    let flags = CloneFlags::from_bits_truncate(flags & !FLAG_MASK);

    info!(
        "sys_clone <= flags: {:?}, exit_signal: {}, stack: {:#x}",
        flags, _exit_signal, stack
    );

    let curr = current();
    let mut new_task = new_user_task(curr.name());

    // TODO: check CLONE_SETTLS
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    new_task
        .ctx_mut()
        .set_tls(axhal::arch::read_thread_pointer().into());

    let trap_frame = read_trapframe_from_kstack(curr.get_kernel_stack_top().unwrap());
    let mut new_uctx = UspaceContext::from(&trap_frame);
    if stack != 0 {
        new_uctx.set_sp(stack);
    }
    // Skip current instruction
    #[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
    {
        let new_uctx_ip = new_uctx.ip();
        new_uctx.set_ip(new_uctx_ip + 4);
    }
    new_uctx.set_retval(0);

    let tid = new_task.id().as_u64() as Pid;
    if flags.contains(CloneFlags::CLONE_THREAD) {
        if !flags.contains(CloneFlags::CLONE_VM | CloneFlags::CLONE_SIGHAND) {
            return Err(LinuxError::EINVAL);
        }

        // create a new thread
        let thread = ThreadBuilder::new(tid, curr.task_ext().thread.process().clone())
            .data(ThreadData::new())
            .build();
        add_thread_to_table(&thread);

        new_task.init_task_ext(TaskExt::new(new_uctx, thread));
    } else {
        // create a new process
        let parent = if flags.contains(CloneFlags::CLONE_PARENT) {
            curr.task_ext()
                .thread
                .process()
                .parent()
                .ok_or(LinuxError::EINVAL)?
        } else {
            curr.task_ext().thread.process().clone()
        };

        let aspace = if flags.contains(CloneFlags::CLONE_VM) {
            curr.task_ext().process_data().aspace.clone()
        } else {
            let mut aspace = curr.task_ext().process_data().aspace.lock();
            let mut aspace = aspace.clone_or_err()?;
            copy_from_kernel(&mut aspace)?;
            Arc::new(Mutex::new(aspace))
        };
        new_task
            .ctx_mut()
            .set_page_table_root(aspace.lock().page_table_root());

        let process_data = ProcessData::new(
            curr.task_ext().process_data().exe_path.read().clone(),
            aspace,
        );

        if flags.contains(CloneFlags::CLONE_FILES) {
            FD_TABLE
                .deref_from(&process_data.ns)
                .init_shared(FD_TABLE.share());
        } else {
            FD_TABLE
                .deref_from(&process_data.ns)
                .init_new(FD_TABLE.copy_inner());
        }

        if flags.contains(CloneFlags::CLONE_FS) {
            CURRENT_DIR
                .deref_from(&process_data.ns)
                .init_shared(CURRENT_DIR.share());
            CURRENT_DIR_PATH
                .deref_from(&process_data.ns)
                .init_shared(CURRENT_DIR_PATH.share());
        } else {
            CURRENT_DIR
                .deref_from(&process_data.ns)
                .init_new(CURRENT_DIR.copy_inner());
            CURRENT_DIR_PATH
                .deref_from(&process_data.ns)
                .init_new(CURRENT_DIR_PATH.copy_inner());
        }

        let process = ProcessBuilder::new(tid)
            .data(process_data)
            .parent(parent)
            .build();

        let thread = ThreadBuilder::new(tid, process.clone())
            .data(ThreadData::new())
            .build();
        add_thread_to_table(&thread);

        new_task.init_task_ext(TaskExt::new(new_uctx, thread));
    }
    axtask::spawn_task(new_task);

    Ok(tid as _)
}

fn read_trapframe_from_kstack(kstack_top: usize) -> TrapFrame {
    let trap_frame_size = core::mem::size_of::<TrapFrame>();
    let trap_frame_ptr = (kstack_top - trap_frame_size) as *mut TrapFrame;
    unsafe { *trap_frame_ptr }
}
