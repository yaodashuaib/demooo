use std::time::Duration;

pub const TIMEOUT: Duration = Duration::from_secs(8);
/// WorkBuddy 多 UA 重试时用更短超时。
pub const SHORT_TIMEOUT: Duration = Duration::from_secs(3);

pub fn get_json(
    url: &str,
    headers: &[(&str, &str)],
) -> Result<(u16, serde_json::Value), String> {
    let mut req = ureq::get(url).timeout(TIMEOUT);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    match req.call() {
        Ok(resp) => {
            let status = resp.status();
            let value = resp
                .into_json::<serde_json::Value>()
                .map_err(|e| e.to_string())?;
            Ok((status, value))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            let value = serde_json::from_str(&body).unwrap_or(serde_json::json!({ "raw": body }));
            Ok((code, value))
        }
        Err(err) => Err(err.to_string()),
    }
}

pub fn post_json(
    url: &str,
    headers: &[(&str, &str)],
    body: &serde_json::Value,
) -> Result<(u16, serde_json::Value), String> {
    post_json_timeout(url, headers, body, TIMEOUT)
}

pub fn post_json_timeout(
    url: &str,
    headers: &[(&str, &str)],
    body: &serde_json::Value,
    timeout: Duration,
) -> Result<(u16, serde_json::Value), String> {
    let mut req = ureq::post(url).timeout(timeout);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    match req.send_json(body.clone()) {
        Ok(resp) => {
            let status = resp.status();
            let value = resp
                .into_json::<serde_json::Value>()
                .map_err(|e| e.to_string())?;
            Ok((status, value))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            let value = serde_json::from_str(&text).unwrap_or(serde_json::json!({ "raw": text }));
            Ok((code, value))
        }
        Err(err) => Err(err.to_string()),
    }
}

pub fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
}

pub fn jwt_claim(token: &str, key: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let padded = match payload.len() % 4 {
        2 => format!("{payload}=="),
        3 => format!("{payload}="),
        _ => payload.to_owned(),
    };
    let decoded = base64::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        payload,
    )
    .or_else(|_| {
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &padded)
    })
    .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    json.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .or_else(|| {
            json.get("https://api.openai.com/auth")
                .and_then(|auth| auth.get(key))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
}
