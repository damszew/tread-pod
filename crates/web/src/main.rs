//! tread-pod config UI: pick a treadmill and edit settings.
//!
//! Talks to the `/api` routes using the `api` crate's types, so client and firmware share one
//! definition of every message.

mod client;

use std::time::Duration;

use api::{Settings, Treadmill};
use leptos::prelude::*;

/// How often the treadmill list (RSSI, live speed) refreshes. Replaced by WS /api/status later.
const REFRESH: Duration = Duration::from_secs(2);

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    view! {
        <h1>"tread-pod"</h1>
        <TreadmillPicker />
        <SettingsForm />
    }
}

#[component]
fn TreadmillPicker() -> impl IntoView {
    let (tick, set_tick) = signal(0u32);
    set_interval(move || set_tick.update(|t| *t = t.wrapping_add(1)), REFRESH);

    let treadmills = LocalResource::new(move || {
        tick.track();
        client::treadmills()
    });
    let select = Action::new_local(|address: &api::Address| {
        let address = *address;
        async move { client::select(address).await }
    });
    // Refresh right after a selection instead of waiting for the next tick.
    Effect::new(move || {
        if select.value().get().is_some() {
            set_tick.update(|t| *t = t.wrapping_add(1));
        }
    });

    view! {
        <h2>"Treadmills"</h2>
        // Transition (not Suspense) keeps the current list visible while it refreshes.
        <Transition fallback=|| view! { <p class="muted">"Scanning…"</p> }>
            {move || {
                treadmills
                    .get()
                    .map(|result| match result {
                        Ok(list) if list.is_empty() => {
                            view! { <p class="muted">"No treadmills in range."</p> }.into_any()
                        }
                        Ok(list) => {
                            view! {
                                <ul>
                                    {list
                                        .into_iter()
                                        .map(|t| view! { <TreadmillRow treadmill=t select /> })
                                        .collect_view()}
                                </ul>
                            }
                                .into_any()
                        }
                        Err(err) => view! { <p class="error">{err}</p> }.into_any(),
                    })
            }}
        </Transition>
    }
}

#[component]
fn TreadmillRow(
    treadmill: Treadmill,
    select: Action<api::Address, Result<(), String>>,
) -> impl IntoView {
    let Treadmill {
        address,
        name,
        rssi,
        speed,
        selected,
    } = treadmill;
    let speed = speed.map_or_else(
        || "–".to_owned(),
        |s| format!("{}.{:02} km/h", s / 100, s % 100),
    );
    view! {
        <li class:selected=selected>
            <span class="name">{name.to_string()}</span>
            <span class="muted">{format!("{rssi} dBm")}</span>
            <span>{speed}</span>
            <button disabled=selected on:click=move |_| { select.dispatch(address); }>
                "Use"
            </button>
        </li>
    }
}

#[component]
fn SettingsForm() -> impl IntoView {
    let settings = LocalResource::new(client::settings);
    let save = Action::new_local(|settings: &Settings| {
        let settings = *settings;
        async move { client::put_settings(settings).await }
    });

    view! {
        <h2>"Settings"</h2>
        <Suspense fallback=|| ()>
            {move || {
                settings
                    .get()
                    .map(|result| match result {
                        Ok(current) => {
                            view! {
                                <label>
                                    <input
                                        type="checkbox"
                                        prop:checked=current.cadence
                                        on:change=move |ev| {
                                            save.dispatch(Settings {
                                                cadence: event_target_checked(&ev),
                                                ..current
                                            });
                                        }
                                    />
                                    " Send estimated cadence"
                                </label>
                                <p class="muted">
                                    "Off by default: estimated cadence can skew running dynamics."
                                </p>
                            }
                                .into_any()
                        }
                        Err(err) => view! { <p class="error">{err}</p> }.into_any(),
                    })
            }}
        </Suspense>
        {move || {
            save.value().get().and_then(Result::err).map(|err| view! { <p class="error">{err}</p> })
        }}
    }
}
