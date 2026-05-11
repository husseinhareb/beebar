use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{Module, ModuleChrome, ModuleView};
use crate::renderer::primitives::TextStyle;

const DEFAULT_FORMAT: &str = "↓ {download} ↑ {upload}";
const UNKNOWN_VALUE: &str = "--";
const LOOPBACK_INTERFACE: &str = "lo";
const ROUTE_TABLE_PATH: &str = "/proc/net/route";
const NETWORK_SYSFS_DIR: &str = "/sys/class/net";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NetworkCounters {
    rx_bytes: u64,
    tx_bytes: u64,
}

#[derive(Debug, Clone)]
struct NetworkSample {
    interface: String,
    counters: NetworkCounters,
    captured_at: Instant,
}

pub struct NetworkModule {
    configured_interface: Option<String>,
    active_interface: Option<String>,
    format: String,
    download_bytes_per_second: Option<u64>,
    upload_bytes_per_second: Option<u64>,
    last_sample: Option<NetworkSample>,
    chrome: ModuleChrome,
    logged_refresh_error: bool,
}

impl NetworkModule {
    pub fn new(interface: Option<String>, format: Option<String>, chrome: ModuleChrome) -> Self {
        Self {
            configured_interface: normalize_optional_string(interface),
            active_interface: None,
            format: resolve_format(format),
            download_bytes_per_second: None,
            upload_bytes_per_second: None,
            last_sample: None,
            chrome,
            logged_refresh_error: false,
        }
    }

    fn apply_sample(&mut self, interface: String, counters: NetworkCounters, captured_at: Instant) {
        let rates = self.last_sample.as_ref().and_then(|previous| {
            if previous.interface != interface {
                return None;
            }

            let elapsed = captured_at.saturating_duration_since(previous.captured_at);
            let elapsed_seconds = elapsed.as_secs_f64();
            if elapsed_seconds <= f64::EPSILON {
                return None;
            }

            Some((
                (counters.rx_bytes.saturating_sub(previous.counters.rx_bytes) as f64
                    / elapsed_seconds)
                    .round() as u64,
                (counters.tx_bytes.saturating_sub(previous.counters.tx_bytes) as f64
                    / elapsed_seconds)
                    .round() as u64,
            ))
        });

        self.active_interface = Some(interface.clone());
        self.download_bytes_per_second = rates.map(|(download, _)| download);
        self.upload_bytes_per_second = rates.map(|(_, upload)| upload);
        self.last_sample = Some(NetworkSample {
            interface,
            counters,
            captured_at,
        });
    }

    fn reset_state(&mut self, interface: Option<String>) {
        self.active_interface = interface;
        self.download_bytes_per_second = None;
        self.upload_bytes_per_second = None;
        self.last_sample = None;
    }

    fn render_text(&self) -> String {
        let interface = self
            .active_interface
            .as_deref()
            .or(self.configured_interface.as_deref())
            .unwrap_or(UNKNOWN_VALUE);
        let download = self
            .download_bytes_per_second
            .map(format_bytes_per_second)
            .unwrap_or_else(|| UNKNOWN_VALUE.to_string());
        let upload = self
            .upload_bytes_per_second
            .map(format_bytes_per_second)
            .unwrap_or_else(|| UNKNOWN_VALUE.to_string());

        self.format
            .replace("{interface}", interface)
            .replace("{download}", &download)
            .replace("{upload}", &upload)
    }
}

impl Module for NetworkModule {
    fn update_interval(&self) -> std::time::Duration {
        self.chrome
            .update_interval
            .unwrap_or(std::time::Duration::from_secs(1))
    }

    fn update(&mut self) {
        let Some(interface) = resolve_interface(self.configured_interface.as_deref()) else {
            if !self.logged_refresh_error {
                log::warn!("network module: no network interface available to monitor");
                self.logged_refresh_error = true;
            }
            self.reset_state(None);
            return;
        };

        match read_interface_counters(&interface) {
            Ok(counters) => {
                self.apply_sample(interface, counters, Instant::now());
                self.logged_refresh_error = false;
            }
            Err(error) => {
                if !self.logged_refresh_error {
                    log::warn!(
                        "network module: failed to refresh interface '{}': {}",
                        interface,
                        error
                    );
                    self.logged_refresh_error = true;
                }
                self.reset_state(Some(interface));
            }
        }
    }

    fn view(&self) -> ModuleView {
        self.chrome.apply(ModuleView {
            text: self.render_text(),
            style: TextStyle::default(),
            ..Default::default()
        })
    }
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn resolve_format(format: Option<String>) -> String {
    match format {
        Some(format) if !format.trim().is_empty() => format,
        _ => DEFAULT_FORMAT.to_string(),
    }
}

fn resolve_interface(configured: Option<&str>) -> Option<String> {
    configured
        .and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .or_else(detect_default_interface)
        .or_else(detect_first_non_loopback_interface)
}

fn detect_default_interface() -> Option<String> {
    fs::read_to_string(ROUTE_TABLE_PATH)
        .ok()
        .and_then(|contents| parse_default_route_interface(&contents))
}

fn parse_default_route_interface(contents: &str) -> Option<String> {
    contents.lines().skip(1).find_map(|line| {
        let mut fields = line.split_whitespace();
        let interface = fields.next()?;
        let destination = fields.next()?;

        if destination == "00000000" && interface != LOOPBACK_INTERFACE {
            Some(interface.to_string())
        } else {
            None
        }
    })
}

fn detect_first_non_loopback_interface() -> Option<String> {
    let mut interfaces: Vec<String> = fs::read_dir(NETWORK_SYSFS_DIR)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name != LOOPBACK_INTERFACE)
        .collect();

    interfaces.sort_unstable();
    interfaces.into_iter().next()
}

fn read_interface_counters(interface: &str) -> io::Result<NetworkCounters> {
    let statistics_dir = Path::new(NETWORK_SYSFS_DIR)
        .join(interface)
        .join("statistics");

    Ok(NetworkCounters {
        rx_bytes: read_counter(&statistics_dir.join("rx_bytes"))?,
        tx_bytes: read_counter(&statistics_dir.join("tx_bytes"))?,
    })
}

fn read_counter(path: &PathBuf) -> io::Result<u64> {
    let raw = fs::read_to_string(path)?;
    raw.trim().parse::<u64>().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid counter in {}: {}", path.display(), error),
        )
    })
}

fn format_bytes_per_second(bytes_per_second: u64) -> String {
    const UNITS: [&str; 5] = ["B/s", "KiB/s", "MiB/s", "GiB/s", "TiB/s"];

    let mut value = bytes_per_second as f64;
    let mut unit_index = 0usize;

    while value >= 1024.0 && unit_index + 1 < UNITS.len() {
        value /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{bytes_per_second} {}", UNITS[unit_index])
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit_index])
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit_index])
    } else {
        format!("{value:.2} {}", UNITS[unit_index])
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::core::module::{ModuleChrome, ModuleView};

    fn default_chrome() -> ModuleChrome {
        ModuleChrome {
            foreground: None,
            background: None,
            padding: (8.0, 8.0),
            icon_spacing: None,
            update_interval: None,
        }
    }

    fn read_view_text(module: &NetworkModule) -> String {
        let ModuleView { text, .. } = module.view();
        text
    }

    #[test]
    fn parses_default_route_interface_from_proc_route() {
        let route_table = "\
Iface Destination Gateway Flags RefCnt Use Metric Mask MTU Window IRTT
wlan0 00000000 0101A8C0 0003 0 0 600 00000000 0 0 0
wlan0 0001A8C0 00000000 0001 0 0 600 00FFFFFF 0 0 0
";

        assert_eq!(
            parse_default_route_interface(route_table),
            Some("wlan0".to_string())
        );
    }

    #[test]
    fn formats_network_speeds_with_binary_units() {
        assert_eq!(format_bytes_per_second(999), "999 B/s");
        assert_eq!(format_bytes_per_second(1_536), "1.50 KiB/s");
        assert_eq!(format_bytes_per_second(5 * 1024 * 1024), "5.00 MiB/s");
    }

    #[test]
    fn keeps_user_format_whitespace() {
        let module = NetworkModule::new(
            None,
            Some("  {download} / {upload}  ".to_string()),
            default_chrome(),
        );

        assert_eq!(read_view_text(&module), "  -- / --  ");
    }

    #[test]
    fn renders_placeholder_tokens() {
        let mut module = NetworkModule::new(
            Some("eth0".to_string()),
            Some("{interface}: ↓ {download} ↑ {upload}".to_string()),
            default_chrome(),
        );
        let start = Instant::now();

        module.apply_sample(
            "eth0".to_string(),
            NetworkCounters {
                rx_bytes: 1_024,
                tx_bytes: 2_048,
            },
            start,
        );
        module.apply_sample(
            "eth0".to_string(),
            NetworkCounters {
                rx_bytes: 4_096,
                tx_bytes: 8_192,
            },
            start + Duration::from_secs(1),
        );

        assert_eq!(read_view_text(&module), "eth0: ↓ 3.00 KiB/s ↑ 6.00 KiB/s");
    }

    #[test]
    fn resets_speed_when_interface_changes() {
        let mut module = NetworkModule::new(None, None, default_chrome());
        let start = Instant::now();

        module.apply_sample(
            "wlan0".to_string(),
            NetworkCounters {
                rx_bytes: 2_048,
                tx_bytes: 4_096,
            },
            start,
        );
        module.apply_sample(
            "eth0".to_string(),
            NetworkCounters {
                rx_bytes: 8_192,
                tx_bytes: 16_384,
            },
            start + Duration::from_secs(1),
        );

        assert_eq!(read_view_text(&module), "↓ -- ↑ --");
    }
}
