use core::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use alloc::sync::Arc;
use axerrno::{LinuxError, LinuxResult};
use axio::PollState;
use axnet::{TcpSocket, UdpSocket};
use axsync::Mutex;

use crate::ctypes::{AF_INET, in_addr, sockaddr_in, stat};

use super::FileLike;

pub enum Socket {
    Udp(Mutex<UdpSocket>),
    Tcp(Mutex<TcpSocket>),
}

macro_rules! impl_socket {
    ($pub:vis fn $name:ident(&self $(,$arg:ident: $arg_ty:ty)*) -> $ret:ty) => {
        $pub fn $name(&self, $($arg: $arg_ty),*) -> $ret {
            match self {
                Socket::Udp(udpsocket) => Ok(udpsocket.lock().$name($($arg),*)?),
                Socket::Tcp(tcpsocket) => Ok(tcpsocket.lock().$name($($arg),*)?),
            }
        }
    };
}

impl Socket {
    pub fn recv(&self, buf: &mut [u8]) -> LinuxResult<usize> {
        match self {
            Socket::Udp(udpsocket) => Ok(udpsocket.lock().recv_from(buf).map(|e| e.0)?),
            Socket::Tcp(tcpsocket) => Ok(tcpsocket.lock().recv(buf)?),
        }
    }

    pub fn sendto(&self, buf: &[u8], addr: SocketAddr) -> LinuxResult<usize> {
        match self {
            // diff: must bind before sendto
            Socket::Udp(udpsocket) => Ok(udpsocket.lock().send_to(buf, addr)?),
            Socket::Tcp(_) => Err(LinuxError::EISCONN),
        }
    }

    pub fn recvfrom(&self, buf: &mut [u8]) -> LinuxResult<(usize, Option<SocketAddr>)> {
        match self {
            // diff: must bind before recvfrom
            Socket::Udp(udpsocket) => Ok(udpsocket
                .lock()
                .recv_from(buf)
                .map(|res| (res.0, Some(res.1)))?),
            Socket::Tcp(tcpsocket) => Ok(tcpsocket.lock().recv(buf).map(|res| (res, None))?),
        }
    }

    pub fn listen(&self) -> LinuxResult {
        match self {
            Socket::Udp(_) => Err(LinuxError::EOPNOTSUPP),
            Socket::Tcp(tcpsocket) => Ok(tcpsocket.lock().listen()?),
        }
    }

    pub fn accept(&self) -> LinuxResult<TcpSocket> {
        match self {
            Socket::Udp(_) => Err(LinuxError::EOPNOTSUPP),
            Socket::Tcp(tcpsocket) => Ok(tcpsocket.lock().accept()?),
        }
    }

    impl_socket!(pub fn send(&self, buf: &[u8]) -> LinuxResult<usize>);
    impl_socket!(pub fn poll(&self) -> LinuxResult<PollState>);
    impl_socket!(pub fn local_addr(&self) -> LinuxResult<SocketAddr>);
    impl_socket!(pub fn peer_addr(&self) -> LinuxResult<SocketAddr>);
    impl_socket!(pub fn bind(&self, addr: SocketAddr) -> LinuxResult);
    impl_socket!(pub fn connect(&self, addr: SocketAddr) -> LinuxResult);
    impl_socket!(pub fn shutdown(&self) -> LinuxResult);
}

impl FileLike for Socket {
    fn read(&self, buf: &mut [u8]) -> LinuxResult<usize> {
        self.recv(buf)
    }

    fn write(&self, buf: &[u8]) -> LinuxResult<usize> {
        self.send(buf)
    }

    fn stat(&self) -> LinuxResult<stat> {
        // not really implemented
        let st_mode = 0o140000 | 0o777u32; // S_IFSOCK | rwxrwxrwx
        Ok(stat {
            st_ino: 1,
            st_nlink: 1,
            st_mode,
            st_uid: 1000,
            st_gid: 1000,
            st_blksize: 4096,
            ..Default::default()
        })
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn core::any::Any + Send + Sync> {
        self
    }

    fn poll(&self) -> LinuxResult<PollState> {
        self.poll()
    }

    fn set_nonblocking(&self, nonblock: bool) -> LinuxResult {
        match self {
            Socket::Udp(udpsocket) => udpsocket.lock().set_nonblocking(nonblock),
            Socket::Tcp(tcpsocket) => tcpsocket.lock().set_nonblocking(nonblock),
        }
        Ok(())
    }
}

impl From<SocketAddrV4> for sockaddr_in {
    fn from(addr: SocketAddrV4) -> sockaddr_in {
        sockaddr_in {
            sin_family: AF_INET as u16,
            sin_port: addr.port().to_be(),
            sin_addr: in_addr {
                // `s_addr` is stored as BE on all machines and the array is in BE order.
                // So the native endian conversion method is used so that it's never swapped.
                s_addr: u32::from_ne_bytes(addr.ip().octets()),
            },
            sin_zero: [0; 8],
        }
    }
}

impl From<sockaddr_in> for SocketAddrV4 {
    fn from(addr: sockaddr_in) -> SocketAddrV4 {
        SocketAddrV4::new(
            Ipv4Addr::from(addr.sin_addr.s_addr.to_ne_bytes()),
            u16::from_be(addr.sin_port),
        )
    }
}
