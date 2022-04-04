use super::event_loop::EventLoop;
use log::{debug, warn};
use nix::errno::Errno;
use nix::sys::epoll::EpollFlags;
use nix::sys::socket::{accept4, connect, setsockopt, sockopt};
use nix::sys::socket::{getpeername, shutdown, socket, Shutdown};
use nix::sys::socket::{AddressFamily, InetAddr, SockAddr, SockFlag, SockProtocol, SockType};
use nix::unistd::{read, write};
use std::net::{SocketAddr, TcpListener};
use std::os::unix::prelude::AsRawFd;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

pub type ConnRef = Arc<Mutex<Connection>>;
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum State {
    Reading,
    Ready,
    Writing,
    Finished,
    Closed,
}

const READABLE: u8 = 0b0001;
const WRITABLE: u8 = 0b0010;

trait EventSet {
    fn is_readable(&self) -> bool;
    fn is_writeable(&self) -> bool;
    fn is_close(&self) -> bool;
    fn is_error(&self) -> bool;
    fn is_hup(&self) -> bool;
}
impl EventSet for EpollFlags {
    fn is_readable(&self) -> bool {
        (*self & (EpollFlags::EPOLLIN | EpollFlags::EPOLLPRI)).bits() > 0
    }
    fn is_writeable(&self) -> bool {
        (*self & EpollFlags::EPOLLOUT).bits() > 0
    }
    fn is_close(&self) -> bool {
        (*self & EpollFlags::EPOLLHUP).bits() > 0 && !((*self & EpollFlags::EPOLLIN).bits() > 0)
    }
    fn is_error(&self) -> bool {
        (*self & EpollFlags::EPOLLERR).bits() > 0
    }
    fn is_hup(&self) -> bool {
        (*self & EpollFlags::EPOLLHUP).bits() > 0
    }
}

#[derive(Debug, Clone)]
pub struct Connection {
    fd: i32,
    state: State,
    write_buf: Vec<u8>,
    read_buf: Vec<u8>,
}

impl Connection {
    pub fn new(fd: i32) -> Self {
        Connection {
            fd,
            state: State::Ready,
            write_buf: Vec::new(),
            read_buf: Vec::new(),
        }
    }
    pub fn bind(addr: &str) -> (i32, TcpListener) {
        let listener = TcpListener::bind(addr).unwrap();
        (listener.as_raw_fd(), listener)
    }
    pub fn connect(addr: &str) -> Connection {
        let sockfd = socket(
            AddressFamily::Inet,
            SockType::Stream,
            SockFlag::SOCK_CLOEXEC,
            SockProtocol::Tcp,
        )
        .unwrap();

        let addr = SocketAddr::from_str(addr).unwrap();
        let inet_addr = InetAddr::from_std(&addr);
        let sock_addr = SockAddr::new_inet(inet_addr);
        // TODO: add a exception handle
        match connect(sockfd, &sock_addr) {
            Ok(()) => debug!("a new connection: {}", sockfd),
            Err(e) => warn!("connect failed: {}", e),
        }
        return Connection::new(sockfd);
    }
    pub fn accept(listen_fd: i32) -> Self {
        let fd = accept4(listen_fd, SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK).unwrap();
        setsockopt(fd, sockopt::TcpNoDelay, &true).unwrap();
        Connection::new(fd)
    }

    pub fn connected(&self) -> bool {
        self.state != State::Closed
    }

    pub fn dispatch(&mut self, revents: EpollFlags) -> State {
        self.state = State::Ready;
        if revents.is_readable() {
            self.read();
        }
        if revents.is_writeable() {
            self.write();
        }
        if revents.is_error() {
            self.state = State::Closed;
        }
        if revents.is_close() {
            self.state = State::Closed;
        }
        return self.state;
    }
    pub fn get_fd(&self) -> i32 {
        self.fd
    }
    pub fn get_state(&self) -> State {
        self.state
    }
    pub fn get_msg(&mut self) -> Vec<u8> {
        let buf = self.read_buf.to_owned();
        self.read_buf.clear();
        buf
    }
    pub fn register_read(&mut self, event_loop: &mut EventLoop) {
        self.read_buf.clear();
        event_loop.reregister(
            self.fd,
            EpollFlags::EPOLLHUP
                | EpollFlags::EPOLLERR
                | EpollFlags::EPOLLIN
                | EpollFlags::EPOLLOUT
                | EpollFlags::EPOLLET,
        );
    }
    pub fn deregister(&self, event_loop: &mut EventLoop) {
        event_loop.deregister(self.fd);
        self.shutdown();
    }
    pub fn shutdown(&self) {
        match shutdown(self.fd, Shutdown::Both) {
            Ok(()) => (),
            Err(e) => warn!("Shutdown {} occur {} error", self.fd, e),
        }
    }
    pub fn send(&mut self, buf: &[u8]) {
        match write(self.fd, buf) {
            Ok(_) => (),
            Err(e) => warn!("send data error: {}", e),
        };
    }
    pub fn write_buf(&mut self, buf: &[u8]) {

        // TODO:
    }
    pub fn write(&mut self) {
        // TODO:
        // write(self.fd, &data).unwrap();
    }
    pub fn get_peer_address(&self) -> SockAddr {
        let addr = getpeername(self.fd).expect("get peer socket address failed");
        addr
    }
    pub fn read(&mut self) {
        let mut buf = [0u8; 1024];
        while self.state != State::Finished && self.state != State::Closed {
            match read(self.fd, &mut buf) {
                Ok(0) => self.state = State::Finished,
                Ok(n) => {
                    self.read_buf.extend_from_slice(&buf[0..n]);
                    self.state = State::Reading;
                    if n != buf.len() {
                        self.state = State::Finished;
                        debug!("Read data len: {}", n);
                        break;
                    }
                }
                Err(Errno::EINTR) => debug!("Read EINTR error"),
                Err(Errno::EAGAIN) => debug!("Read EAGIN error"),
                Err(e) => {
                    self.state = State::Closed;
                    warn!("Read error: {}", e);
                }
            }
            // TODO: buffer replace vec
            if self.write_buf.len() >= 64 * 1024 {
                self.state = State::Reading;
                debug!("Send data size exceed 64kB");
                break;
            }
        }
    }
}