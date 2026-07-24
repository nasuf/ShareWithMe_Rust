use reqwest::redirect::Policy;

use crate::analysis::is_safe_remote_url;

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
