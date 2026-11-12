//! Typed calls to the `/api` routes.

use api::{Address, Select, Settings, Treadmills};
use gloo_net::http::{Request, Response};

/// Errors are shown to the user as-is, so they're plain strings.
type Result<T> = std::result::Result<T, String>;

pub async fn treadmills() -> Result<Treadmills> {
    json(Request::get("/api/treadmills").send().await).await
}

pub async fn select(address: Address) -> Result<()> {
    let request = Request::post("/api/select")
        .json(&Select { address })
        .map_err(|e| e.to_string())?;
    ok(request.send().await)
}

pub async fn settings() -> Result<Settings> {
    json(Request::get("/api/settings").send().await).await
}

pub async fn put_settings(settings: Settings) -> Result<()> {
    let request = Request::put("/api/settings")
        .json(&settings)
        .map_err(|e| e.to_string())?;
    ok(request.send().await)
}

async fn json<T: serde::de::DeserializeOwned>(
    response: std::result::Result<Response, gloo_net::Error>,
) -> Result<T> {
    let response = checked(response)?;
    response.json().await.map_err(|e| e.to_string())
}

fn ok(response: std::result::Result<Response, gloo_net::Error>) -> Result<()> {
    checked(response).map(drop)
}

fn checked(response: std::result::Result<Response, gloo_net::Error>) -> Result<Response> {
    let response = response.map_err(|e| format!("bridge unreachable: {e}"))?;
    if response.ok() {
        Ok(response)
    } else {
        Err(format!("{} {}", response.status(), response.status_text()))
    }
}
