//! Serves the real `/api` router on 127.0.0.1:8080 with made-up treadmills, so the web UI can be
//! developed without hardware. `trunk serve` proxies `/api` here (see `crates/web/Trunk.toml`).

use std::{
    cell::RefCell,
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

use api::{AppHandle, Select, Settings, Treadmill, Treadmills};

const ADDR: &str = "127.0.0.1:8080";

#[derive(Clone)]
struct Mock(Rc<RefCell<MockState>>);

struct MockState {
    settings: Settings,
    selected: usize,
}

const FAKES: [(&str, [u8; 6], i8); 3] = [
    ("Gym Treadmill 1", [0xc0, 0xde, 0x00, 0x00, 0x00, 0x01], -48),
    ("Gym Treadmill 2", [0xc0, 0xde, 0x00, 0x00, 0x00, 0x02], -63),
    ("Gym Treadmill 3", [0xc0, 0xde, 0x00, 0x00, 0x00, 0x03], -79),
];

impl AppHandle for Mock {
    fn treadmills(&self) -> Treadmills {
        let state = self.0.borrow();
        // Wobble speed and RSSI a little so the UI has something live to show.
        let wobble = i8::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                % 5,
        )
        .unwrap_or(0);
        FAKES
            .iter()
            .enumerate()
            .map(|(i, &(name, address, rssi))| Treadmill {
                address,
                name: name.try_into().expect("fake name fits"),
                rssi: rssi - wobble,
                speed: (i == state.selected).then(|| 1000 + 10 * u16::from(wobble.unsigned_abs())),
                selected: i == state.selected,
            })
            .collect()
    }

    fn select(&self, select: Select) -> bool {
        match FAKES.iter().position(|f| f.1 == select.address) {
            Some(i) => {
                self.0.borrow_mut().selected = i;
                true
            }
            None => false,
        }
    }

    fn settings(&self) -> Settings {
        self.0.borrow().settings
    }

    fn set_settings(&self, settings: Settings) {
        self.0.borrow_mut().settings = settings;
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    let mock = Mock(Rc::new(RefCell::new(MockState {
        settings: Settings::from(&engine::config::Config::default()),
        selected: 0,
    })));
    let app = Rc::new(api::router::<Mock>().with_state(mock));
    let config = Rc::new(picoserve::Config::const_default());

    let listener = tokio::net::TcpListener::bind(ADDR).await?;
    println!("api dev server listening on http://{ADDR}");

    tokio::task::LocalSet::new()
        .run_until(async {
            loop {
                let (socket, peer) = listener.accept().await?;
                let (app, config) = (app.clone(), config.clone());
                tokio::task::spawn_local(async move {
                    let mut buffer = [0; 2048];
                    if let Err(err) = picoserve::Server::new_tokio(&app, &config, &mut buffer)
                        .serve(socket)
                        .await
                    {
                        eprintln!("{peer}: {err:?}");
                    }
                });
            }
        })
        .await
}
