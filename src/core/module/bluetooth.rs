//! Bluetooth module.
//!
//! Queries the bluez system-bus service for adapter + connected-device state
//! and renders an icon + short status string (e.g. "󰂲 no-controller",
//! "󰂯 on", "󰂯 SomeHeadset"). When the bluez daemon is not running it
//! reports "no-controller" — matching waybar's behaviour.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::runtime::Runtime;
use zbus::{Connection, fdo};
use zbus::zvariant::OwnedValue;

use super::{Module, ModuleChrome, ModuleView};
use crate::renderer::primitives::TextStyle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BluetoothState {
    /// bluez daemon not reachable / no adapter known.
    NoController,
    /// Adapter present but powered off (rfkill or `Powered=false`).
    Off,
    /// Adapter powered on, no Connected device.
    On,
    /// Adapter powered on with one or more connected devices.
    Connected(String),
}

pub struct BluetoothModule {
    state: Arc<Mutex<BluetoothState>>,
    chrome: ModuleChrome,
    icon_on: String,
    icon_off: String,
    icon_no_controller: String,
    /// Format template. Placeholders: {icon}, {status}.
    format: Option<String>,
    /// Tokio runtime kept alive for the background polling task.
    _rt: Arc<Runtime>,
}

impl BluetoothModule {
    pub fn new(
        format: Option<String>,
        icon_on: Option<String>,
        icon_off: Option<String>,
        icon_no_controller: Option<String>,
        chrome: ModuleChrome,
        poll_interval: Duration,
    ) -> Self {
        let state = Arc::new(Mutex::new(BluetoothState::NoController));

        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("beebar-bluetooth")
                .enable_all()
                .build()
                .expect("failed to build tokio runtime for bluetooth"),
        );

        {
            let state_clone = state.clone();
            rt.spawn(async move {
                run_poller(state_clone, poll_interval).await;
            });
        }

        Self {
            state,
            chrome,
            icon_on: icon_on.unwrap_or_else(|| "󰂯".to_string()),
            icon_off: icon_off.unwrap_or_else(|| "󰂲".to_string()),
            icon_no_controller: icon_no_controller.unwrap_or_else(|| "󰂲".to_string()),
            format,
            _rt: rt,
        }
    }
}

impl Module for BluetoothModule {
    fn update(&mut self) {
        // Polling runs in the background; nothing to do here.
    }

    fn update_interval(&self) -> std::time::Duration {
        // update() is a no-op — the real polling runs on a dedicated tokio
        // task. This interval only governs how often we wake the bar.
        self.chrome
            .update_interval
            .unwrap_or(std::time::Duration::from_secs(1))
    }

    fn view(&self) -> ModuleView {
        let state = self.state.lock().unwrap().clone();
        let (icon, status) = match state {
            BluetoothState::NoController => (&self.icon_no_controller, "no-controller".to_string()),
            BluetoothState::Off => (&self.icon_off, "off".to_string()),
            BluetoothState::On => (&self.icon_on, "on".to_string()),
            BluetoothState::Connected(name) => (&self.icon_on, name),
        };

        let text = match &self.format {
            Some(fmt) => fmt
                .replace("{icon}", icon)
                .replace("{status}", &status),
            None => format!("{icon} {status}"),
        };

        self.chrome.apply(ModuleView {
            text,
            style: TextStyle::default(),
            ..Default::default()
        })
    }
}

async fn run_poller(state: Arc<Mutex<BluetoothState>>, poll_interval: Duration) {
    loop {
        let next = match poll_once().await {
            Ok(s) => s,
            Err(error) => {
                log::debug!("[bluetooth] poll failed: {error}");
                BluetoothState::NoController
            }
        };

        {
            let mut guard = state.lock().unwrap();
            if *guard != next {
                *guard = next;
            }
        }

        tokio::time::sleep(poll_interval).await;
    }
}

async fn poll_once() -> zbus::Result<BluetoothState> {
    let conn = Connection::system().await?;

    // Cheapest probe: ask the bus daemon whether anyone owns org.bluez.
    let dbus = fdo::DBusProxy::new(&conn).await?;
    let owned = dbus
        .name_has_owner("org.bluez".try_into().map_err(|e: zbus::names::Error| {
            zbus::Error::Variant(zbus::zvariant::Error::Message(e.to_string()))
        })?)
        .await
        .unwrap_or(false);

    if !owned {
        return Ok(BluetoothState::NoController);
    }

    // Enumerate adapters and devices via ObjectManager.
    let om = fdo::ObjectManagerProxy::builder(&conn)
        .destination("org.bluez")?
        .path("/")?
        .build()
        .await?;

    let objects = match om.get_managed_objects().await {
        Ok(o) => o,
        Err(_) => return Ok(BluetoothState::NoController),
    };

    let mut any_adapter = false;
    let mut any_powered = false;
    let mut connected_alias: Option<String> = None;

    for (_path, ifaces) in &objects {
        if let Some(adapter) = ifaces.get("org.bluez.Adapter1") {
            any_adapter = true;
            if get_bool(adapter, "Powered").unwrap_or(false) {
                any_powered = true;
            }
        }
        if let Some(device) = ifaces.get("org.bluez.Device1") {
            if get_bool(device, "Connected").unwrap_or(false) {
                if connected_alias.is_none() {
                    connected_alias = get_string(device, "Alias")
                        .or_else(|| get_string(device, "Name"));
                }
            }
        }
    }

    if !any_adapter {
        Ok(BluetoothState::NoController)
    } else if !any_powered {
        Ok(BluetoothState::Off)
    } else if let Some(alias) = connected_alias {
        Ok(BluetoothState::Connected(alias))
    } else {
        Ok(BluetoothState::On)
    }
}

fn get_bool(
    props: &std::collections::HashMap<String, OwnedValue>,
    key: &str,
) -> Option<bool> {
    props.get(key).and_then(|v| bool::try_from(v).ok())
}

fn get_string(
    props: &std::collections::HashMap<String, OwnedValue>,
    key: &str,
) -> Option<String> {
    props
        .get(key)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| String::try_from(v).ok())
        .filter(|s| !s.is_empty())
}
