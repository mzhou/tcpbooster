use std::os::unix::io::{AsRawFd, FromRawFd};

use clap::{App, Arg};
use maligned::A4k;
use nix::sys::socket::{
    bind, setsockopt, socket,
    sockopt::{IpTransparent, ReuseAddr},
    AddressFamily, InetAddr, SockAddr, SockFlag, SockType,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn forward_data<F: AsyncReadExt + Unpin, T: AsyncWriteExt + Unpin>(
    log_tag: &str,
    buf: &mut [u8],
    from: &mut F,
    to: &mut T,
) {
    loop {
        match from.read(buf).await {
            Ok(0) => {
                println!("{}read EOF", log_tag);
                break;
            }
            Ok(n) => {
                //println!("{}read {} bytes", log_tag, n);
                match to.write_all(&buf[..n]).await {
                    Ok(()) => {}
                    Err(e) => {
                        println!("{}write_all fail {}", log_tag, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("{}read fail {}", log_tag, e);
                break;
            }
        }
    }
    to.shutdown();
}

fn create_bound_socket(addr: &std::net::SocketAddr) -> nix::Result<std::net::TcpStream> {
    let sock = unsafe {
        std::net::TcpStream::from_raw_fd(socket(
            AddressFamily::Inet,
            SockType::Stream,
            SockFlag::empty(),
            None,
        )?)
    };
    setsockopt(sock.as_raw_fd(), IpTransparent, &true)?;
    setsockopt(sock.as_raw_fd(), ReuseAddr, &true)?;
    bind(sock.as_raw_fd(), &SockAddr::Inet(InetAddr::from_std(addr)))?;
    return Ok(sock);
}

fn create_unbound_socket() -> nix::Result<std::net::TcpStream> {
    let sock = unsafe {
        std::net::TcpStream::from_raw_fd(socket(
            AddressFamily::Inet,
            SockType::Stream,
            SockFlag::empty(),
            None,
        )?)
    };
    setsockopt(sock.as_raw_fd(), ReuseAddr, &true)?;
    return Ok(sock);
}

async fn accept_loop_inner(
    mut listener: TcpListener,
    banned_port: u16,
    default_bind: bool,
) -> Result<(), tokio::io::Error> {
    loop {
        let (in_sock, in_remote_addr) = listener.accept().await?;
        let out_remote_addr = in_sock.local_addr()?;
        if out_remote_addr.port() == banned_port {
            println!("deny direct port {} connection", banned_port);
            continue;
        }
        let in_to_out_log_tag = format!("{}->{} ", in_remote_addr, out_remote_addr);
        println!("{}accept", in_to_out_log_tag);
        tokio::spawn(async move {
            if let Err(e) = in_sock.set_nodelay(true) {
                println!("{}in set_nodelay failed {}", in_to_out_log_tag, e);
            }
            match if default_bind {
                create_unbound_socket()
            } else {
                create_bound_socket(&in_remote_addr)
            } {
                Ok(std_out_sock) => {
                    match TcpStream::connect_std(std_out_sock, &out_remote_addr).await {
                        Ok(out_sock) => {
                            println!("{}connect success", in_to_out_log_tag);
                            if let Err(e) = out_sock.set_nodelay(true) {
                                println!("{}out set_nodelay failed {}", in_to_out_log_tag, e);
                            }
                            let (mut in_read, mut in_write) = in_sock.into_split();
                            let (mut out_read, mut out_write) = out_sock.into_split();

                            tokio::spawn(async move {
                                let mut buf = A4k::default();
                                println!("{}buf {:p}", &in_to_out_log_tag, &buf);
                                forward_data(
                                    &in_to_out_log_tag,
                                    &mut buf,
                                    &mut in_read,
                                    &mut out_write,
                                )
                                .await;
                            });

                            tokio::spawn(async move {
                                let mut buf = A4k::default();
                                let out_to_in_log_tag =
                                    format!("{}<-{} ", in_remote_addr, out_remote_addr);
                                println!("{}buf {:p}", &out_to_in_log_tag, &buf);
                                forward_data(
                                    &out_to_in_log_tag,
                                    &mut buf,
                                    &mut out_read,
                                    &mut in_write,
                                )
                                .await;
                            });
                        }
                        Err(e) => {
                            println!("{}connect failed {}", in_to_out_log_tag, e);
                        }
                    }
                }
                Err(e) => {
                    println!("{}bind failed {}", in_to_out_log_tag, e);
                }
            }
        });
    }
}

async fn accept_loop(listener: TcpListener, banned_port: u16, default_bind: bool) {
    let _ = accept_loop_inner(listener, banned_port, default_bind).await;
}

const DEFAULT_BIND: &str = "DEFAULT_BIND";
const PORT1: &str = "PORT1";
const PORT2: &str = "PORT2";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let matches = App::new("tcpbooster")
        .arg(
            Arg::with_name(DEFAULT_BIND)
                .help("Don't bind outgoing connection to original source address and port")
                .long("default-bind")
                .short("d"),
        )
        .arg(
            Arg::with_name(PORT1)
                .default_value("1")
                .help("Port 1")
                .long("port1")
                .short("1")
                .takes_value(true),
        )
        .arg(
            Arg::with_name(PORT2)
                .default_value("2")
                .help("Port 2")
                .long("port2")
                .short("1")
                .takes_value(true),
        )
        .get_matches();
    let default_bind = matches.occurrences_of(DEFAULT_BIND) != 0;
    let port1: u16 = matches.value_of(PORT1).unwrap().parse().unwrap();
    let port2: u16 = matches.value_of(PORT2).unwrap().parse().unwrap();

    let listener1 = TcpListener::bind(format!("0.0.0.0:{}", port1)).await?;
    setsockopt(listener1.as_raw_fd(), IpTransparent, &true)?;
    let task1 = tokio::spawn(accept_loop(listener1, port1, default_bind));

    let listener2 = TcpListener::bind(format!("0.0.0.0:{}", port2)).await?;
    setsockopt(listener2.as_raw_fd(), IpTransparent, &true)?;
    let task2 = tokio::spawn(accept_loop(listener2, port2, default_bind));

    let _ = task1.await;
    let _ = task2.await;

    return Ok(());
}
