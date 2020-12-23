use std::error::Error;
use std::fmt;
use std::os::unix::io::AsRawFd;

use clap::{App, Arg};
use maligned::A4k;
use nix::sys::socket::{
    setsockopt,
    sockopt::IpTransparent,
};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket, TcpStream};

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
    let _ = to.shutdown().await;
}

#[derive(Debug)]
enum SocketError {
    Nix(nix::Error),
    StdIo(std::io::Error),
}

impl From<nix::Error> for SocketError {
    fn from(e: nix::Error) -> SocketError {
        Self::Nix(e)
    }
}

impl From<std::io::Error> for SocketError {
    fn from(e: std::io::Error) -> SocketError {
        Self::StdIo(e)
    }
}

impl fmt::Display for SocketError {
    fn fmt(self: &Self, f: &mut fmt::Formatter) -> fmt::Result {
        use SocketError::*;
        write!(f, "{}", match self {
            Nix(e) => format!("Nix({})", e),
            StdIo(e) => format!("StdIo({})", e),
        })
    }
}

impl Error for SocketError {
}

fn create_bound_socket(addr: std::net::SocketAddr) -> Result<TcpSocket, SocketError> {
    let sock = TcpSocket::new_v6()?;
    setsockopt(sock.as_raw_fd(), IpTransparent, &true)?;
    sock.set_reuseaddr(true)?;
    sock.bind(addr)?;
    return Ok(sock);
}

#[derive(Clone)]
struct SocksConfig {
    server: SocketAddr,
    username: String,
    password: String,
}

#[derive(Clone)]
struct Config {
    bind: Option<SocketAddr>,
    socks: Option<SocksConfig>,
}

// TODO: proper error bubbling
async fn create_outgoing_socket(
    socks: Option<SocksConfig>,
    bind_addr: &std::net::SocketAddr,
    connect_addr: &std::net::SocketAddr,
) -> Result<TcpStream, Box<dyn Error>> {
    let std_out_sock = create_bound_socket(*bind_addr)?;
    if let Some(socks) = socks {
        let tokio_out_sock = std_out_sock.connect(socks.server).await?;
        tokio_socks::tcp::Socks5Stream::connect_with_password_and_socket(
            tokio_out_sock,
            connect_addr,
            socks.username.as_str(),
            socks.password.as_str(),
        )
        .await
        .map(|s| s.into_inner())
        .map_err(|e| e.into())
    } else {
        std_out_sock.connect(*connect_addr).await.map_err(|e| e.into())
    }
}

async fn accept_loop_inner(
    listener: TcpListener,
    banned_port: u16,
    config: Config,
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
        tokio::spawn({
            let config = config.clone();
            async move {
                if let Err(e) = in_sock.set_nodelay(true) {
                    println!("{}in set_nodelay failed {}", in_to_out_log_tag, e);
                }
                let bind_addr = config.bind.unwrap_or(in_remote_addr);
                match create_outgoing_socket(config.socks, &bind_addr, &out_remote_addr).await {
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
        });
    }
}

async fn accept_loop(listener: TcpListener, banned_port: u16, config: Config) {
    let _ = accept_loop_inner(listener, banned_port, config).await;
}

const BIND: &str = "BIND";
const PORT1: &str = "PORT1";
const PORT2: &str = "PORT2";
const SOCKS_SERVER: &str = "SOCKS_SERVER";
const SOCKS_USERNAME: &str = "SOCKS_USERNAME";
const SOCKS_PASSWORD: &str = "SOCKS_PASSWORD";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let matches = App::new("tcpbooster")
        .arg(
            Arg::with_name(BIND)
                .help("Bind outgoing connections to this address")
                .long("bind")
                .short("b")
                .takes_value(true),
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
        .arg(
            Arg::with_name(SOCKS_PASSWORD)
                .help("SOCKS password")
                .long("socks-password")
                .takes_value(true),
        )
        .arg(
            Arg::with_name(SOCKS_SERVER)
                .help("SOCKS server")
                .long("socks-server")
                .takes_value(true),
        )
        .arg(
            Arg::with_name(SOCKS_USERNAME)
                .help("SOCKS username")
                .long("socks-username")
                .takes_value(true),
        )
        .get_matches();
    let bind: Option<SocketAddr> = if let Some(v) = matches.value_of(BIND) {
        Some(v.parse()?)
    } else {
        None
    };
    println!("bind {:?} {:?}", matches.value_of(BIND), bind);
    let port1: u16 = matches.value_of(PORT1).unwrap().parse().unwrap();
    let port2: u16 = matches.value_of(PORT2).unwrap().parse().unwrap();
    let socks_password = matches.value_of(SOCKS_PASSWORD);
    let socks_server: Option<SocketAddr> = matches
        .value_of(SOCKS_SERVER)
        .map(|v| v.parse())
        .transpose()?;
    let socks_username = matches.value_of(SOCKS_USERNAME);

    let socks_config = if let (Some(socks_password), Some(socks_server), Some(socks_username)) =
        (socks_password, socks_server, socks_username)
    {
        Some(SocksConfig {
            password: String::from(socks_password),
            server: socks_server,
            username: String::from(socks_username),
        })
    } else {
        None
    };
    let config = Config {
        bind,
        socks: socks_config,
    };

    let listener1 = TcpListener::bind(format!("0.0.0.0:{}", port1)).await?;
    setsockopt(listener1.as_raw_fd(), IpTransparent, &true)?;
    let task1 = tokio::spawn(accept_loop(listener1, port1, config.clone()));

    let listener2 = TcpListener::bind(format!("0.0.0.0:{}", port2)).await?;
    setsockopt(listener2.as_raw_fd(), IpTransparent, &true)?;
    let task2 = tokio::spawn(accept_loop(listener2, port2, config.clone()));

    let _ = task1.await;
    let _ = task2.await;

    Ok(())
}
