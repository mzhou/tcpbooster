use std::os::unix::io::AsRawFd;

use maligned::A4k;
use nix::sys::socket::{setsockopt, sockopt::IpTransparent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn forward_data<F: AsyncReadExt+Unpin, T: AsyncWriteExt+Unpin>(log_tag: &str, buf: &mut [u8], from: &mut F, to: &mut T) {
    loop {
        match from.read(buf).await {
            Ok(0) => {
                println!("{}read EOF", log_tag);
                let _ = to.shutdown();
                return;

            },
            Ok(n) => {
                //println!("{}read {} bytes", log_tag, n);
                match to.write_all(&buf[..n]).await {
                    Ok(()) => {
                        //println!("{}write_all success", log_tag);
                    },
                    _ => {
                        println!("{}write_all fail", log_tag);
                        return;
                    },
                }
            },
            _ => {
                println!("{}read fail", log_tag);
                let _ = to.shutdown();
                return;
            },
        }
    }
}

async fn accept_loop_inner(mut listener: TcpListener, banned_port: u16) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        let (in_sock, in_remote_addr) = listener.accept().await?;
        let out_remote_addr = in_sock.local_addr()?;
        if out_remote_addr.port() == banned_port {
            println!("deny direct port {} connection", banned_port);
            continue;
        }
        let in_to_out_log_tag = format!("{}->{} ", in_remote_addr, out_remote_addr);
        println!("{}accept", in_to_out_log_tag);
        let out_socket2 = socket2::Socket::new(socket2::Domain::ipv4(), socket2::Type::stream(), None)?;
        setsockopt(out_socket2.as_raw_fd(), IpTransparent, &true)?;
        out_socket2.bind(&socket2::SockAddr::from(in_remote_addr))?;
        tokio::spawn(async move {
            match TcpStream::connect_std(out_socket2.into_tcp_stream(), &out_remote_addr).await {
                Ok(out_sock) => {
                    println!("{}connect success", in_to_out_log_tag);
                    let (mut in_read, mut in_write) = in_sock.into_split();
                    let (mut out_read, mut out_write) = out_sock.into_split();

                    tokio::spawn(async move {
                        let mut buf =  A4k::default();
                        println!("{}buf {:p}", &in_to_out_log_tag, &buf);
                        forward_data(&in_to_out_log_tag, &mut buf, &mut in_read, &mut out_write).await;
                    });

                    tokio::spawn(async move {
                        let mut buf = A4k::default();
                        let out_to_in_log_tag = format!("{}<-{} ", in_remote_addr, out_remote_addr);
                        println!("{}buf {:p}", &out_to_in_log_tag, &buf);
                        forward_data(&out_to_in_log_tag, &mut buf, &mut out_read, &mut in_write).await;
                    });
                },
                _ => {
                    println!("{}connect failed", in_to_out_log_tag);
                },
            }
            return ()
        });
    }
}

async fn accept_loop(listener: TcpListener, banned_port: u16) {
    let _ = accept_loop_inner(listener, banned_port).await;
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener1 = TcpListener::bind("0.0.0.0:1").await?;
    setsockopt(listener1.as_raw_fd(), IpTransparent, &true)?;
    tokio::spawn(accept_loop(listener1, 1));

    let listener2 = TcpListener::bind("0.0.0.0:2").await?;
    setsockopt(listener2.as_raw_fd(), IpTransparent, &true)?;
    tokio::spawn(accept_loop(listener2, 2));

    loop {
    }
}
