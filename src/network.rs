use reqwest::redirect::Policy;

use crate::links::is_safe_remote_url;

pub(crate) fn safe_redirect_policy(max_redirects: usize) -> Policy {
    Policy::custom(move |attempt| {
        if attempt.previous().len() >= max_redirects {
            attempt.error("too many redirects")
        } else if !is_safe_remote_url(attempt.url()) {
            attempt.stop()
        } else {
            attempt.follow()
        }
    })
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::safe_redirect_policy;

    /// A server whose every response redirects to `location`.
    async fn redirect_to(location: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buffer = [0_u8; 1024];
                let _ = socket.read(&mut buffer).await;
                let response = format!(
                    "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        format!("http://{addr}/")
    }

    #[tokio::test]
    async fn does_not_follow_redirects_into_private_networks() {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(safe_redirect_policy(8))
            .build()
            .unwrap();
        let url = redirect_to("http://10.0.0.1/admin").await;
        let response = client.get(&url).send().await.unwrap();
        // The redirect is returned as-is instead of being followed.
        assert_eq!(response.status(), 302);
        assert_eq!(response.url().as_str(), url);
    }
}
