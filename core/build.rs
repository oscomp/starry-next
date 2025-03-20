use std::path::PathBuf;

fn main() {
    const ALLOW_TYPES: &[&str] = &[
        "stat",
        "size_t",
        "ssize_t",
        "off_t",
        "mode_t",
        "sock.*",
        "fd_set",
        "timeval",
        "pthread_t",
        "pthread_attr_t",
        "pthread_mutex_t",
        "pthread_mutexattr_t",
        "epoll_event",
        "iovec",
        "clockid_t",
        "rlimit",
        "aibuf",
    ];
    const ALLOW_VARS: &[&str] = &[
        "CLOCK_.*",
        "O_.*",
        "AF_.*",
        "SOCK_.*",
        "IPPROTO_.*",
        "FD_.*",
        "F_.*",
        "_SC_.*",
        "EPOLL_CTL_.*",
        "EPOLL.*",
        "RLIMIT_.*",
        "EAI_.*",
        "MAXADDRS",
        "AT_.*",
    ];

    let mut builder = bindgen::builder()
        .header("ctypes.h")
        .clang_arg("-I./include")
        .derive_default(true)
        .wrap_unsafe_ops(true)
        .ignore_functions()
        .use_core()
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));

    for ty in ALLOW_TYPES {
        builder = builder.allowlist_type(ty);
    }
    for var in ALLOW_VARS {
        builder = builder.allowlist_var(var);
    }

    builder
        .generate()
        .expect("Unable to generate bindings")
        .write_to_file(PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ctypes.rs"))
        .expect("Couldn't write bindings!");
}
