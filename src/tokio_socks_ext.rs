use tokio::io::{AsyncRead, AsyncWrite};
use tokio_socks::tcp::Socks5Stream;
use tokio_socks::{IntoTargetAddr, Result};

pub async fn auto_connect_with_password_and_socket<'a, 't, S, T>(
    socket: S,
    target: T,
    username: &'a str,
    password: &'a str,
) -> Result<Socks5Stream<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
    T: IntoTargetAddr<'t>,
{
    if !username.is_empty() || !password.is_empty() {
        Socks5Stream::<S>::connect_with_password_and_socket(socket, target, username, password)
            .await
    } else {
        Socks5Stream::<S>::connect_with_socket(socket, target).await
    }
}
