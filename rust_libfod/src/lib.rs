// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use fod_rust_runtime::FOD_VERSION_LABEL;
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::Command;
use std::sync::OnceLock;

static VERSION_LABEL_CSTR: OnceLock<CString> = OnceLock::new();

struct FodProgram {
    name: &'static [u8],
    binary: &'static [u8],
    env_override: &'static str,
    description: &'static [u8],
}

const PROGRAMS: &[FodProgram] = &[
    FodProgram {
        name: b"fod-bootstrap\0",
        binary: b"fod-bootstrap\0",
        env_override: "FOD_LIBFOD_FOD_BOOTSTRAP_BIN",
        description: b"FOD mount bootstrap command\0",
    },
    FodProgram {
        name: b"mkfs.fod\0",
        binary: b"mkfs.fod\0",
        env_override: "FOD_LIBFOD_MKFS_BIN",
        description: b"FOD schema administration command\0",
    },
    FodProgram {
        name: b"fod-config\0",
        binary: b"fod-config\0",
        env_override: "FOD_LIBFOD_FOD_CONFIG_BIN",
        description: b"FOD runtime configuration command\0",
    },
    FodProgram {
        name: b"fod-change\0",
        binary: b"fod-change\0",
        env_override: "FOD_LIBFOD_FOD_CHANGE_BIN",
        description: b"FOD live runtime change command\0",
    },
    FodProgram {
        name: b"fod-indexer\0",
        binary: b"fod-indexer\0",
        env_override: "FOD_LIBFOD_FOD_INDEXER_BIN",
        description: b"FOD source indexing command\0",
    },
    FodProgram {
        name: b"fod-monitor\0",
        binary: b"fod-monitor\0",
        env_override: "FOD_LIBFOD_FOD_MONITOR_BIN",
        description: b"FOD observability command\0",
    },
    FodProgram {
        name: b"fod-rust-fuse\0",
        binary: b"fod-rust-fuse\0",
        env_override: "FOD_LIBFOD_FOD_RUST_FUSE_BIN",
        description: b"FOD FUSE daemon command\0",
    },
    FodProgram {
        name: b"mount.fod\0",
        binary: b"mount.fod\0",
        env_override: "FOD_LIBFOD_MOUNT_FOD_BIN",
        description: b"FOD mount helper command\0",
    },
];

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::new("").expect("empty CString"));
}

fn static_c_char_ptr(bytes: &'static [u8]) -> *const c_char {
    bytes.as_ptr().cast()
}

fn set_last_error(message: impl AsRef<str>) {
    let sanitized = message.as_ref().replace('\0', "\\0");
    let c_string = CString::new(sanitized).unwrap_or_else(|_| {
        CString::new("libfod error message contained NUL").expect("literal CString")
    });
    LAST_ERROR.with(|last_error| {
        *last_error.borrow_mut() = c_string;
    });
}

fn clear_last_error() {
    LAST_ERROR.with(|last_error| {
        *last_error.borrow_mut() = CString::new("").expect("empty CString");
    });
}

fn cstr_to_str(cstr: &CStr) -> Result<&str, c_int> {
    cstr.to_str().map_err(|_| {
        set_last_error("argument is not valid UTF-8");
        -2
    })
}

fn str_from_nul_bytes(bytes: &'static [u8]) -> &'static str {
    let payload = &bytes[..bytes.len().saturating_sub(1)];
    std::str::from_utf8(payload).expect("static libfod strings are UTF-8")
}

fn program_name(program: &FodProgram) -> &'static str {
    str_from_nul_bytes(program.name)
}

fn program_binary(program: &FodProgram) -> &'static str {
    str_from_nul_bytes(program.binary)
}

fn find_program_index(name: &str) -> Option<usize> {
    PROGRAMS
        .iter()
        .position(|program| program_name(program) == name || program_binary(program) == name)
}

fn program_at(index: usize) -> Option<&'static FodProgram> {
    PROGRAMS.get(index)
}

fn collect_args(argc: c_int, argv: *const *const c_char) -> Result<Vec<String>, c_int> {
    if argc < 0 {
        set_last_error("argc must not be negative");
        return Err(-2);
    }
    if argc == 0 {
        return Ok(Vec::new());
    }
    if argv.is_null() {
        set_last_error("argv must not be null when argc is greater than zero");
        return Err(-2);
    }
    let mut args = Vec::with_capacity(argc as usize);
    for offset in 0..argc as usize {
        let arg_ptr = unsafe { *argv.add(offset) };
        if arg_ptr.is_null() {
            set_last_error(format!("argv[{offset}] must not be null"));
            return Err(-2);
        }
        let arg = unsafe { CStr::from_ptr(arg_ptr) };
        args.push(cstr_to_str(arg)?.to_owned());
    }
    Ok(args)
}

fn resolved_binary(program: &FodProgram) -> String {
    std::env::var(program.env_override)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| program_binary(program).to_owned())
}

fn run_program(program: &FodProgram, argc: c_int, argv: *const *const c_char) -> c_int {
    let args = match collect_args(argc, argv) {
        Ok(args) => args,
        Err(code) => return code,
    };
    let binary = resolved_binary(program);
    match Command::new(&binary).args(&args).status() {
        Ok(status) => {
            clear_last_error();
            status.code().unwrap_or_else(|| {
                set_last_error(format!("{binary} terminated without an exit code"));
                -5
            })
        }
        Err(error) => {
            set_last_error(format!("failed to run {binary}: {error}"));
            -4
        }
    }
}

fn run_program_by_name(name: *const c_char, argc: c_int, argv: *const *const c_char) -> c_int {
    if name.is_null() {
        set_last_error("program name must not be null");
        return -1;
    }
    let name = unsafe { CStr::from_ptr(name) };
    let name = match cstr_to_str(name) {
        Ok(name) => name,
        Err(code) => return code,
    };
    match find_program_index(name).and_then(program_at) {
        Some(program) => run_program(program, argc, argv),
        None => {
            set_last_error(format!("unknown FOD program: {name}"));
            -3
        }
    }
}

fn run_program_by_index(index: usize, argc: c_int, argv: *const *const c_char) -> c_int {
    match program_at(index) {
        Some(program) => run_program(program, argc, argv),
        None => {
            set_last_error(format!("unknown FOD program index: {index}"));
            -3
        }
    }
}

fn ffi_int_call(call: impl FnOnce() -> c_int) -> c_int {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(code) => code,
        Err(_) => {
            set_last_error("libfod panic crossed the FFI boundary");
            -127
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_version_label() -> *const c_char {
    VERSION_LABEL_CSTR
        .get_or_init(|| CString::new(FOD_VERSION_LABEL).expect("runtime version label has no NUL"))
        .as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_last_error_message() -> *const c_char {
    LAST_ERROR.with(|last_error| last_error.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_program_count() -> usize {
    PROGRAMS.len()
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_program_name(index: usize) -> *const c_char {
    program_at(index)
        .map(|program| static_c_char_ptr(program.name))
        .unwrap_or(std::ptr::null())
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_program_binary(index: usize) -> *const c_char {
    program_at(index)
        .map(|program| static_c_char_ptr(program.binary))
        .unwrap_or(std::ptr::null())
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_program_description(index: usize) -> *const c_char {
    program_at(index)
        .map(|program| static_c_char_ptr(program.description))
        .unwrap_or(std::ptr::null())
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_program_find(name: *const c_char) -> isize {
    if name.is_null() {
        set_last_error("program name must not be null");
        return -1;
    }
    let name = unsafe { CStr::from_ptr(name) };
    let name = match cstr_to_str(name) {
        Ok(name) => name,
        Err(_) => return -1,
    };
    match find_program_index(name) {
        Some(index) => {
            clear_last_error();
            index as isize
        }
        None => {
            set_last_error(format!("unknown FOD program: {name}"));
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_program_run(
    name: *const c_char,
    argc: c_int,
    argv: *const *const c_char,
) -> c_int {
    ffi_int_call(|| run_program_by_name(name, argc, argv))
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_bootstrap(argc: c_int, argv: *const *const c_char) -> c_int {
    ffi_int_call(|| run_program_by_index(0, argc, argv))
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_mkfs(argc: c_int, argv: *const *const c_char) -> c_int {
    ffi_int_call(|| run_program_by_index(1, argc, argv))
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_config(argc: c_int, argv: *const *const c_char) -> c_int {
    ffi_int_call(|| run_program_by_index(2, argc, argv))
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_change(argc: c_int, argv: *const *const c_char) -> c_int {
    ffi_int_call(|| run_program_by_index(3, argc, argv))
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_indexer(argc: c_int, argv: *const *const c_char) -> c_int {
    ffi_int_call(|| run_program_by_index(4, argc, argv))
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_monitor(argc: c_int, argv: *const *const c_char) -> c_int {
    ffi_int_call(|| run_program_by_index(5, argc, argv))
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_rust_fuse(argc: c_int, argv: *const *const c_char) -> c_int {
    ffi_int_call(|| run_program_by_index(6, argc, argv))
}

#[unsafe(no_mangle)]
pub extern "C" fn fod_mount(argc: c_int, argv: *const *const c_char) -> c_int {
    ffi_int_call(|| run_program_by_index(7, argc, argv))
}
