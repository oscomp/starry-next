pub mod ctypes;

use core::alloc::Layout;

use axhal::{arch::TrapFrame, trap::ANY_TRAP};
use axtask::{TaskExtRef, current, exit};
use ctypes::{SignalAction, SignalActionFlags, SignalDisposition, SignalInfo, SignalSet};
use linkme::distributed_slice as register_trap_handler;

use crate::ptr::{PtrWrapper, UserConstPtr, UserPtr};

pub const SIGKILL: u32 = 9;
pub const SIGSTOP: u32 = 19;

#[derive(Debug)]
enum DefaultSignalAction {
    /// Terminate the process.
    Terminate,

    /// Ignore the signal.
    Ignore,

    /// Terminate the process and generate a core dump.
    CoreDump,

    /// Stop the process.
    Stop,

    /// Continue the process if stopped.
    Continue,
}
const DEFAULT_ACTIONS: [DefaultSignalAction; 32] = [
    // Unspecified
    DefaultSignalAction::Ignore,
    // SIGHUP
    DefaultSignalAction::Terminate,
    // SIGINT
    DefaultSignalAction::Terminate,
    // SIGQUIT
    DefaultSignalAction::CoreDump,
    // SIGILL
    DefaultSignalAction::CoreDump,
    // SIGTRAP
    DefaultSignalAction::CoreDump,
    // SIGABRT
    DefaultSignalAction::CoreDump,
    // SIGBUS
    DefaultSignalAction::CoreDump,
    // SIGFPE
    DefaultSignalAction::CoreDump,
    // SIGKILL
    DefaultSignalAction::Terminate,
    // SIGUSR1
    DefaultSignalAction::Terminate,
    // SIGSEGV
    DefaultSignalAction::CoreDump,
    // SIGUSR2
    DefaultSignalAction::Terminate,
    // SIGPIPE
    DefaultSignalAction::Terminate,
    // SIGALRM
    DefaultSignalAction::Terminate,
    // SIGTERM
    DefaultSignalAction::Terminate,
    // SIGSTKFLT
    DefaultSignalAction::Terminate,
    // SIGCHLD
    DefaultSignalAction::Ignore,
    // SIGCONT
    DefaultSignalAction::Continue,
    // SIGSTOP
    DefaultSignalAction::Stop,
    // SIGTSTP
    DefaultSignalAction::Stop,
    // SIGTTIN
    DefaultSignalAction::Stop,
    // SIGTTOU
    DefaultSignalAction::Stop,
    // SIGURG
    DefaultSignalAction::Ignore,
    // SIGXCPU
    DefaultSignalAction::CoreDump,
    // SIGXFSZ
    DefaultSignalAction::CoreDump,
    // SIGVTALRM
    DefaultSignalAction::Terminate,
    // SIGPROF
    DefaultSignalAction::Terminate,
    // SIGWINCH
    DefaultSignalAction::Ignore,
    // SIGIO
    DefaultSignalAction::Terminate,
    // SIGPWR
    DefaultSignalAction::Terminate,
    // SIGSYS
    DefaultSignalAction::CoreDump,
];

/// Structure to manage signal handling in a task.
pub struct SignalManager {
    /// The set of signals currently blocked from delivery.
    pub blocked: SignalSet,

    /// The signal actions.
    actions: [SignalAction; 32],

    /// The pending signals.
    pub pending: SignalSet,
    pending_info: [Option<SignalInfo>; 32],
}
impl SignalManager {
    pub fn new() -> Self {
        Self {
            blocked: SignalSet::default(),
            actions: Default::default(),

            pending: SignalSet::default(),
            pending_info: Default::default(),
        }
    }

    pub fn send_signal(&mut self, sig: SignalInfo) -> bool {
        let signo = sig.signo();
        if !self.pending.add(signo) {
            return false;
        }
        self.pending_info[signo as usize] = Some(sig);
        true
    }

    pub fn set_action(&mut self, signo: u32, action: SignalAction) {
        self.actions[signo as usize] = action;
    }
    pub fn action(&self, signo: u32) -> &SignalAction {
        &self.actions[signo as usize]
    }

    /// Dequeue the next pending signal.
    pub fn dequeue_signal(&mut self) -> Option<SignalInfo> {
        self.pending
            .dequeue(&self.blocked)
            .and_then(|signo| self.pending_info[signo as usize].take())
    }
}

pub struct SignalFrame {
    tf: TrapFrame,
    blocked: SignalSet,
    siginfo: SignalInfo,
}

#[register_trap_handler(ANY_TRAP)]
fn handle_any_trap(tf: &mut TrapFrame, from_user: bool) -> bool {
    if !from_user {
        return false;
    }

    let curr = current();
    let mut sigman = curr.task_ext().signal.lock();
    while let Some(sig) = sigman.dequeue_signal() {
        let signo = sig.signo();
        info!("Handle signal: {}", signo);
        let action = &sigman.actions[signo as usize];
        match action.disposition {
            SignalDisposition::Default => match DEFAULT_ACTIONS[signo as usize] {
                DefaultSignalAction::Terminate => exit(128 + signo as i32),
                DefaultSignalAction::CoreDump => {
                    warn!("Core dump not implemented");
                    exit(128 + signo as i32);
                }
                DefaultSignalAction::Stop => {
                    warn!("Stop not implemented");
                    exit(-1);
                }
                DefaultSignalAction::Ignore => {}
                DefaultSignalAction::Continue => {
                    warn!("Continue not implemented");
                }
            },
            SignalDisposition::Ignore => {}
            SignalDisposition::Handler(handler) => {
                let layout = Layout::new::<SignalFrame>();
                let aligned_sp = (tf.sp() - layout.size()) & !(layout.align() - 1);

                let frame_ptr: UserPtr<SignalFrame> = aligned_sp.into();
                let frame_ptr = frame_ptr.get().expect("invalid frame ptr");
                // SAFETY: pointer is valid
                let frame = unsafe { &mut *frame_ptr };

                *frame = SignalFrame {
                    tf: *tf,
                    blocked: sigman.blocked,
                    siginfo: sig,
                };

                tf.set_ip(handler as _);
                tf.set_sp(aligned_sp);
                tf.set_arg0(signo as _);
                tf.set_arg1(&frame.siginfo as *const _ as _);
                tf.set_arg2(frame_ptr as _);

                let restorer = action.restorer.map_or(0, |f| f as _);
                #[cfg(target_arch = "x86_64")]
                tf.write_ra(restorer);
                #[cfg(not(target_arch = "x86_64"))]
                tf.set_ra(restorer);

                let mut mask = action.mask.clone();
                if !action.flags.contains(SignalActionFlags::NODEFER) {
                    mask.add(signo);
                }
                sigman.blocked.add_from(&mask);
            }
        }
    }

    true
}

pub fn sigreturn(tf: &mut TrapFrame) {
    let frame_ptr: UserConstPtr<SignalFrame> = tf.sp().into();
    let frame = frame_ptr.get().expect("invalid frame ptr");
    // SAFETY: pointer is valid
    let frame = unsafe { &*frame };

    *tf = frame.tf;
    #[cfg(any(
        target_arch = "riscv32",
        target_arch = "riscv64",
        target_arch = "loongarch64"
    ))]
    tf.set_ip(tf.ip() - 4);

    let curr = current();
    let mut sigman = curr.task_ext().signal.lock();
    sigman.blocked = frame.blocked;
}
