use axum::{
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

#[derive(Clone, Debug)]
pub struct AuthConfig {
    username_hash: [u8; 32],
    password_hash: [u8; 32],
}

impl AuthConfig {
    pub fn new(username: &str, password: &str) -> Self {
        Self {
            username_hash: Sha256::digest(username.as_bytes()).into(),
            password_hash: Sha256::digest(password.as_bytes()).into(),
        }
    }

    fn accepts(&self, value: &str) -> bool {
        let Some(encoded) = value.strip_prefix("Basic ") else {
            return false;
        };
        let Ok(decoded) = STANDARD.decode(encoded) else {
            return false;
        };
        let Some(separator) = decoded.iter().position(|byte| *byte == b':') else {
            return false;
        };
        let username_hash: [u8; 32] = Sha256::digest(&decoded[..separator]).into();
        let password_hash: [u8; 32] = Sha256::digest(&decoded[separator + 1..]).into();
        bool::from(
            username_hash.ct_eq(&self.username_hash) & password_hash.ct_eq(&self.password_hash),
        )
    }
}

pub async fn require_basic_auth(
    State(auth): State<AuthConfig>,
    request: Request,
    next: Next,
) -> Response {
    let accepted = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| auth.accepts(value));

    if accepted {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(
                header::WWW_AUTHENTICATE,
                "Basic realm=\"tpad\", charset=\"UTF-8\"",
            )],
            "Authentication required",
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_exact_credentials() {
        let auth = AuthConfig::new("tpad", "correct horse");
        let valid = format!("Basic {}", STANDARD.encode("tpad:correct horse"));
        let wrong = format!("Basic {}", STANDARD.encode("tpad:wrong"));
        assert!(auth.accepts(&valid));
        assert!(!auth.accepts(&wrong));
        assert!(!auth.accepts("Bearer token"));
    }
}
