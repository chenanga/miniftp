use nix::fcntl::{flock, open, FlockArg, OFlag};
use nix::libc::exit;
use nix::sys::resource::*;
use nix::sys::signal::{pthread_sigmask, signal};
use nix::sys::signal::{SigHandler, SigSet, SigmaskHow, Signal};
use nix::sys::stat::{umask, Mode};
use nix::unistd::{chdir, fork, ftruncate, getpid, setsid, write};

const LOCK_FILE: &'static str = "/var/run/miniftp.pid";

pub fn daemonize() {
    umask(Mode::empty());
    getrlimit(Resource::RLIMIT_NOFILE).expect("get trlimit failed!");
    let result = unsafe { fork().expect("cant't fork a new process") };
    if result.is_parent() {
        unsafe { exit(0) };
    }
    unsafe {
        signal(Signal::SIGPIPE, SigHandler::SigIgn).unwrap();
        signal(Signal::SIGHUP, SigHandler::SigIgn).unwrap();
    }
    pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&SigSet::all()), None).unwrap();
    setsid().expect("can't set sid");
    chdir("/").unwrap();
}

pub fn already_runing() -> bool {
    let lock_mode = Mode::S_IRUSR | Mode::S_IWUSR | Mode::S_IRGRP | Mode::S_IROTH;
    let fd = open(LOCK_FILE, OFlag::O_RDWR | OFlag::O_CREAT, lock_mode).unwrap();
    flock(fd, FlockArg::LockExclusiveNonblock).unwrap();
    ftruncate(fd, 0).unwrap();
    let pid = getpid();
    let buf = format!("{}", pid);
    match write(fd, buf.as_bytes()) {
        Ok(0) | Err(_) => return false,
        _ => return true,
    }
}
