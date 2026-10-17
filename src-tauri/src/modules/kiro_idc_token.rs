use reqwest::{Client, RequestBuilder};
use serde::Serialize;

#[derive(Serialize)]
#[serde(tag = "grantType")]
pub(super) enum Grant<'a> {
    #[serde(rename = "urn:ietf:params:oauth:grant-type:device_code")]
    DeviceCode {
        #[serde(rename = "deviceCode")]
        value: &'a str,
    },
    #[serde(rename = "refresh_token")]
    RefreshToken {
        #[serde(rename = "refreshToken")]
        value: &'a str,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TokenRequest<'a> {
    pub client_id: &'a str,
    pub client_secret: &'a str,
    #[serde(flatten)]
    pub grant: Grant<'a>,
}

pub(super) fn request(client: &Client, endpoint: &str, token: TokenRequest<'_>) -> RequestBuilder {
    // AWS CreateToken 使用 JSON 和 camelCase 字段，而非标准 OAuth 表单。
    client.post(endpoint).json(&token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn device_authorization_uses_aws_json_request() {
        let built = request(
            &Client::new(),
            "https://oidc.us-east-1.amazonaws.com/token",
            TokenRequest {
                client_id: "test-client",
                client_secret: "secret+&= 中文",
                grant: Grant::DeviceCode { value: "device+&=" },
            },
        )
        .build()
        .expect("build device request");
        assert_eq!("application/json", built.headers()["content-type"]);
        let actual: Value = serde_json::from_slice(built.body().unwrap().as_bytes().unwrap())
            .expect("JSON request body");
        assert_eq!(
            json!({
                "clientId": "test-client",
                "clientSecret": "secret+&= 中文",
                "grantType": "urn:ietf:params:oauth:grant-type:device_code",
                "deviceCode": "device+&="
            }),
            actual
        );
    }

    #[test]
    fn refresh_uses_aws_json_without_device_code_or_scope() {
        let built = request(
            &Client::new(),
            "https://oidc.us-east-1.amazonaws.com/token",
            TokenRequest {
                client_id: "test-client",
                client_secret: "test-secret",
                grant: Grant::RefreshToken {
                    value: "refresh+&=",
                },
            },
        )
        .build()
        .expect("build refresh request");
        assert_eq!("application/json", built.headers()["content-type"]);
        let actual: Value = serde_json::from_slice(built.body().unwrap().as_bytes().unwrap())
            .expect("JSON request body");
        assert_eq!(
            json!({
                "clientId": "test-client",
                "clientSecret": "test-secret",
                "grantType": "refresh_token",
                "refreshToken": "refresh+&="
            }),
            actual
        );
    }
}
