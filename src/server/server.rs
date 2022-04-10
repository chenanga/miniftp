use crate::handler::session::Session;
use crate::net::connection::Connection;
use crate::net::event_loop::{EventLoop, Handler, Token};
use crate::net::queue::{BlockingQueue, BlockingQueueRef};
use crate::net::sorted_list::TimerList;
use crate::threadpool::threadpool::ThreadPool;
use crate::utils::config::{Config, DEFAULT_CONF_FILE};
use crate::utils::utils::already_running;
use log::{debug, info, warn};
use nix::sys::epoll::EpollFlags;
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::unix::prelude::AsRawFd;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

const DEFAULT_TIME_OUT: u64 = 60; // time (s)

type TaskQueueRef = BlockingQueueRef<Arc<Mutex<Session>>>;

pub struct FtpServer {
    conn_list: TimerList<i32, Rc<RefCell<Connection>>>,
    request_queue: TaskQueueRef,
    worker_pool: ThreadPool,
    sessions: HashMap<i32, Arc<Mutex<Session>>>, // <cmd_fd, session_ref>
    event_loop: EventLoop,
    conn_map: Arc<Mutex<HashMap<i32, i32>>>, // <cmd_fd, data fd>
    config: Config,
}

impl FtpServer {
    pub fn new(config: Config, event_loop: &mut EventLoop) -> Self {
        let q: TaskQueueRef = BlockingQueueRef::new(BlockingQueue::new(64));
        let pool = ThreadPool::new(0);

        for _ in 0..pool.len() {
            let q_clone = q.clone();
            pool.execute(move || loop {
                let session = q_clone.pop_front();
                session.lock().unwrap().handle_command();
            });
        }
        FtpServer {
            conn_list: TimerList::new(DEFAULT_TIME_OUT),
            request_queue: q,
            worker_pool: pool,
            sessions: HashMap::new(),
            event_loop: event_loop.clone(),
            conn_map: Arc::new(Mutex::new(HashMap::new())),
            config,
        }
    }
}

impl Handler for FtpServer {
    type Message = String;
    type Timeout = i32;
    fn ready(&mut self, event_loop: &mut EventLoop, token: Token, revent: EpollFlags) {
        if let Token::Listen(listen_fd) = token {
            let mut conn = Connection::accept(listen_fd);
            let fd = conn.get_fd();
            debug!("A new connection: {:?}:{}", token, fd);

            if self.config.max_clients > self.sessions.len() {
                conn.register_read(event_loop);
                let s = Session::new(&self.config, conn, event_loop, &self.conn_map);
                self.sessions.insert(fd, Arc::new(Mutex::new(s)));
            } else {
                warn!(
                    "Session number: {}, shutdown conn: {}",
                    self.sessions.len(),
                    fd
                );
                conn.shutdown();
            }
        }
    }
    fn notify(&mut self, _event_loop: &mut EventLoop, token: Token, revents: EpollFlags) {
        if let Token::Notify(fd) = token {
            let s = self.sessions.get(&fd).unwrap();
            self.request_queue.push_back(s.clone());
            // TODO: Session 注销逻辑
        } else if let Token::Timer(fd) = token {
            debug!("timer: {}", fd);
            // TODO: 应该定时注销的是 session, 注销一些最不活跃的session
            // let old_len = self.conn_list.len();
            // self.conn_list.remove_idle();
            // let mut _buf = [0u8; 8];
            // read(fd, &mut _buf).unwrap_or_default(); // 读取这个 timer_fd
            // let new_len = self.conn_list.len();
            // if old_len != new_len {
            //     debug!(
            //         "Remove idle connection, old len:{}, new len: {}",
            //         old_len, new_len
            //     );
            // }
        }
    }
}

pub fn run_server() {
    if already_running() {
        warn!("Already running...");
        return;
    }

    let config = Config::new(DEFAULT_CONF_FILE);
    debug!("config: {:?}", config);
    let addr = format!("{}:{}", config.server_addr, config.server_port);
    let (_, listener) = Connection::bind(&addr);
    info!(
        "Start server listener, addr: {}, fd: {:?}",
        addr,
        listener.as_raw_fd()
    );

    let mut event_loop = EventLoop::new(listener);
    let mut ftpserver = FtpServer::new(config, &mut event_loop);
    event_loop.run(&mut ftpserver);
}
