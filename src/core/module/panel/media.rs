//! The panel's media card, fed by `playerctl` (any MPRIS player: browsers,
//! Spotify, mpv, ...), including the track's artwork.

use std::path::{Path, PathBuf};

use super::pages::{app_icon, shell_quote};
use crate::core::module::IconData;

/// Lists every player, one per line, fields separated by tabs.
pub const QUERY: &str = "playerctl -a metadata --format \
    '{{playerInstance}}\t{{status}}\t{{xesam:title}}\t{{xesam:artist}}\t{{position}}\t{{mpris:length}}\t{{mpris:artUrl}}'";

/// Artwork is cropped to a square and scaled to this many pixels a side:
/// twice the size it is drawn at, so it stays sharp on scaled outputs.
const ART_SIZE: u32 = 96;

#[derive(Debug, Clone, PartialEq)]
pub struct MediaState {
    /// playerctl's name for the player, e.g. "firefox.instance_1_21477".
    pub instance: String,
    pub playing: bool,
    pub title: String,
    pub artist: Option<String>,
    /// Seconds into the track.
    pub position: f64,
    /// Track length in seconds, when the player reports it.
    pub length: Option<f64>,
    /// Where the player keeps the track's artwork (file:// or http(s)://).
    pub art_url: Option<String>,
}

impl MediaState {
    pub fn icon(&self) -> &'static str {
        app_icon(&self.instance)
    }

    /// 0.0 to 1.0 through the track, if its length is known.
    pub fn progress(&self) -> Option<f64> {
        self.length
            .filter(|l| *l > 0.0)
            .map(|l| (self.position / l).clamp(0.0, 1.0))
    }

    /// `playerctl` aimed at this player: "previous", "play-pause", "next".
    pub fn command(&self, action: &str) -> String {
        format!("playerctl -p {} {action}", shell_quote(&self.instance))
    }

    /// Jump to `fraction` of the track. `None` without a known length.
    pub fn seek_command(&self, fraction: f64) -> Option<String> {
        let length = self.length.filter(|l| *l > 0.0)?;
        Some(self.command(&format!(
            "position {:.1}",
            fraction.clamp(0.0, 1.0) * length
        )))
    }
}

/// The player to show: the first one playing, else the first paused one.
/// Stopped players and players with nothing loaded are skipped.
pub fn parse(output: &str) -> Option<MediaState> {
    let players: Vec<(String, MediaState)> = output
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            // Players without artwork may print one field less.
            let (instance, status, title, artist, position, length, art) = match fields.as_slice() {
                [i, s, t, a, p, l] => (*i, *s, *t, *a, *p, *l, ""),
                [i, s, t, a, p, l, art] => (*i, *s, *t, *a, *p, *l, *art),
                _ => return None,
            };
            let title = title.trim();
            if instance.trim().is_empty() || title.is_empty() {
                return None;
            }
            let micros = |v: &str| v.trim().parse::<f64>().ok().map(|us| us / 1e6);
            Some((
                status.trim().to_string(),
                MediaState {
                    instance: instance.trim().to_string(),
                    playing: status.trim() == "Playing",
                    title: title.to_string(),
                    artist: Some(artist.trim().to_string()).filter(|a| !a.is_empty()),
                    position: micros(position).unwrap_or(0.0),
                    length: micros(length),
                    art_url: Some(art.trim().to_string()).filter(|a| !a.is_empty()),
                },
            ))
        })
        .collect();
    ["Playing", "Paused"].iter().find_map(|wanted| {
        players
            .iter()
            .find(|(status, _)| status == wanted)
            .map(|(_, state)| state.clone())
    })
}

/// Load the artwork at `url`: a local file (`file://` or a plain path), or
/// a web address fetched with `download(url, destination)`. Returns `None`
/// when it cannot be read or decoded.
pub fn load_art(url: &str, download: impl Fn(&str, &Path) -> bool) -> Option<IconData> {
    if let Some(path) = url.strip_prefix("file://") {
        return decode_art(Path::new(&percent_decode(path)));
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        let temp = download_path();
        if let Some(dir) = temp.parent() {
            std::fs::create_dir_all(dir).ok()?;
        }
        let art = download(url, &temp).then(|| decode_art(&temp)).flatten();
        let _ = std::fs::remove_file(&temp);
        return art;
    }
    url.starts_with('/')
        .then(|| decode_art(Path::new(url)))
        .flatten()
}

/// The `curl` command that fetches `url` into `destination`.
pub fn download_command(url: &str, destination: &Path) -> String {
    format!(
        "curl -fsSL --max-time 5 -o {} {}",
        shell_quote(&destination.to_string_lossy()),
        shell_quote(url)
    )
}

/// Scratch file for downloaded artwork, removed right after decoding.
fn download_path() -> PathBuf {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    cache
        .join("beebar")
        .join(format!("artwork-{}", std::process::id()))
}

/// Decode an image, crop it to a centred square (so 16:9 video thumbnails
/// fill the square instead of being letterboxed) and convert it to the
/// renderer's premultiplied BGRA.
fn decode_art(path: &Path) -> Option<IconData> {
    // Detect the format from the bytes: downloads and some players' cache
    // files have no (or a wrong) extension.
    let img = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let side = img.width().min(img.height());
    if side == 0 {
        return None;
    }
    let img = img
        .crop_imm(
            (img.width() - side) / 2,
            (img.height() - side) / 2,
            side,
            side,
        )
        .resize_exact(ART_SIZE, ART_SIZE, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let (width, height) = img.dimensions();
    let premultiply = |c: u8, a: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
    let pixels = img
        .pixels()
        .flat_map(|p| {
            let [r, g, b, a] = p.0;
            [premultiply(b, a), premultiply(g, a), premultiply(r, a), a]
        })
        .collect();
    Some(IconData {
        pixels,
        width,
        height,
    })
}

/// Undo URL `%XX` escapes (file:// URLs escape spaces and the like).
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(path: &Path, width: u32, height: u32) {
        // Left half red, right half blue, fully opaque.
        let img = image::RgbaImage::from_fn(width, height, |x, _| {
            if x < width / 2 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        img.save(path).unwrap();
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("beebar-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn decodes_file_artwork_into_a_square() {
        let dir = temp_dir("art file");
        let path = dir.join("cover art.png");
        write_png(&path, 64, 32);

        let url = format!("file://{}", path.to_string_lossy().replace(' ', "%20"));
        let art = load_art(&url, |_, _| false).unwrap();
        assert_eq!((art.width, art.height), (ART_SIZE, ART_SIZE));
        assert_eq!(art.pixels.len(), (ART_SIZE * ART_SIZE * 4) as usize);
        // Centre crop of a red|blue image: the left edge is red (BGRA order).
        assert_eq!(&art.pixels[0..4], &[0, 0, 255, 255]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn web_artwork_goes_through_the_downloader() {
        let dir = temp_dir("art web");
        let source = dir.join("source.png");
        write_png(&source, 10, 10);
        let art = load_art("https://example.com/a.png", |url, dest| {
            assert_eq!(url, "https://example.com/a.png");
            std::fs::copy(&source, dest).is_ok()
        });
        assert!(art.is_some());
        assert!(load_art("https://example.com/a.png", |_, _| false).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unreadable_artwork_is_none() {
        assert!(load_art("file:///nonexistent/cover.png", |_, _| false).is_none());
        assert!(load_art("data:image/png;base64,AAAA", |_, _| true).is_none());
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("/a%20b/c%25d"), "/a b/c%d");
        assert_eq!(percent_decode("/bad%zz"), "/bad%zz");
    }

    #[test]
    fn download_command_is_quoted() {
        assert_eq!(
            download_command("https://x/a b.jpg", Path::new("/tmp/c d")),
            "curl -fsSL --max-time 5 -o '/tmp/c d' 'https://x/a b.jpg'"
        );
    }

    #[test]
    fn reads_the_art_url() {
        let output = "spotify\tPlaying\tSong\tArtist\t0\t1000000\thttps://i.scdn.co/image/ab67\n";
        assert_eq!(
            parse(output).unwrap().art_url.as_deref(),
            Some("https://i.scdn.co/image/ab67")
        );
    }

    #[test]
    fn prefers_the_playing_player() {
        let output = "firefox.instance_1\tPaused\tSong A\tArtist A\t1000000\t4000000\n\
                      spotify\tPlaying\tSong B\t\t2000000\t\n\
                      mpv\tStopped\tOld\t\t0\t0\n";
        let state = parse(output).unwrap();
        assert_eq!(state.instance, "spotify");
        assert!(state.playing);
        assert_eq!(state.artist, None);
        assert_eq!(state.progress(), None);
    }

    #[test]
    fn falls_back_to_a_paused_player() {
        let output = "firefox.instance_1_21477\tPaused\tGirl, so confusing\tCharli xcx, Lorde\t51250000\t205000000\n";
        let state = parse(output).unwrap();
        assert!(!state.playing);
        assert_eq!(state.title, "Girl, so confusing");
        assert_eq!(state.artist.as_deref(), Some("Charli xcx, Lorde"));
        assert_eq!(state.progress(), Some(0.25));
        assert_eq!(
            state.command("play-pause"),
            "playerctl -p 'firefox.instance_1_21477' play-pause"
        );
        assert_eq!(
            state.seek_command(0.5).unwrap(),
            "playerctl -p 'firefox.instance_1_21477' position 102.5"
        );
    }

    #[test]
    fn nothing_to_show() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("No players found\n"), None);
        assert_eq!(parse("mpv\tStopped\tOld\t\t0\t0\n"), None);
    }
}
