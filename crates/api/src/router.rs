use picoserve::{
    extract::{Json, State},
    response::{NoContent, StatusCode},
    routing::{PathRouter, get, post},
};

use crate::{Select, Settings, Treadmills};

/// What the router needs from the running bridge. The firmware implements this over its shared
/// state; the dev server implements it with made-up data.
///
/// Methods are synchronous: implementations read a snapshot or queue a command, they never wait
/// on BLE.
pub trait AppHandle: Clone {
    fn treadmills(&self) -> Treadmills;

    /// Switches to the treadmill with this address. Returns `false` if it's unknown.
    fn select(&self, select: Select) -> bool;

    fn settings(&self) -> Settings;

    fn set_settings(&self, settings: Settings);
}

/// `204 No Content` without a body (a bare [`StatusCode`] response always writes one, which
/// browsers reject for 204).
const NO_CONTENT: (StatusCode, NoContent) = (StatusCode::NO_CONTENT, NoContent);

/// The `/api` routes. Callers attach state with [`picoserve::Router::with_state`] and add their
/// own routes (e.g. static assets).
pub fn router<H: AppHandle>() -> picoserve::Router<impl PathRouter<H>, H> {
    picoserve::Router::new()
        .route(
            "/api/treadmills",
            get(async |State(app): State<H>| picoserve::response::Json(app.treadmills())),
        )
        .route(
            "/api/select",
            post(async |State(app): State<H>, Json(select): Json<Select>| {
                if app.select(select) {
                    Ok(NO_CONTENT)
                } else {
                    Err(StatusCode::NOT_FOUND)
                }
            }),
        )
        .route(
            "/api/settings",
            get(async |State(app): State<H>| picoserve::response::Json(app.settings())).put(
                async |State(app): State<H>, Json(settings): Json<Settings>| {
                    app.set_settings(settings);
                    NO_CONTENT
                },
            ),
        )
}
