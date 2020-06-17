use std::io;
use std::net::SocketAddr;

use maligned::A4k;
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

#[tokio::main]
async fn main() -> io::Result<()> {
    let mut listener = TcpListener::bind("0.0.0.0:65535").await?;

    loop {
        let (in_sock, in_remote_addr) = listener.accept().await?;
        match "127.0.0.1:80".parse::<SocketAddr>() {
            Ok(out_remote_addr) => {
                let in_to_out_log_tag = format!("{}->{} ", in_remote_addr, out_remote_addr);
                println!("{}accept", in_to_out_log_tag);
                tokio::spawn(async move {
                    match TcpStream::connect(out_remote_addr).await {
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

            },
            _ => (),
        }
    }
}
