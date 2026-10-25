//! What the web version does differently: browsers can't save files where
//! you choose, so songs and exports are downloaded, files are picked with
//! the browser's upload dialog, and the open song is kept in the browser's
//! local storage so it's there next visit.

#[cfg(target_arch = "wasm32")]
pub use browser::*;

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Local storage only holds text, so songs are stored as base64.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(BASE64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut n, mut bits) = (0u32, 0);
    for c in text.bytes().filter(|&c| c != b'=') {
        let v = BASE64.iter().position(|&b| b == c)? as u32;
        n = n << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((n >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::sync::mpsc::Sender;

    use eframe::egui;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    /// A file the user picked in the browser.
    pub enum Picked {
        Song(String, Vec<u8>),
        Sample(String, Vec<u8>),
    }

    const SONG_KEY: &str = "daw-song";
    const NAME_KEY: &str = "daw-song-name";

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    /// The song kept from last visit: its file name and contents.
    pub fn stored_song() -> Option<(String, Vec<u8>)> {
        let storage = storage()?;
        let bytes = super::base64_decode(&storage.get_item(SONG_KEY).ok()??)?;
        let name = storage.get_item(NAME_KEY).ok()?;
        Some((name.unwrap_or_default(), bytes))
    }

    /// Keeps the song for next visit. Fails if it's too big for the
    /// browser (usually about 5 MB, so long samples may not fit).
    pub fn store_song(name: &str, bytes: &[u8]) -> Result<(), String> {
        let storage = storage().ok_or("no browser storage")?;
        let full = "This song is too big to keep in the browser; use File > Save to keep it.";
        storage
            .set_item(SONG_KEY, &super::base64_encode(bytes))
            .map_err(|_| full.to_string())?;
        let _ = storage.set_item(NAME_KEY, name);
        Ok(())
    }

    /// Hands a file to the browser to save, like a download.
    pub fn download(name: &str, bytes: &[u8], mime: &str) -> Result<(), String> {
        let fail = |_| format!("Couldn't download {name}");
        let array = js_sys::Uint8Array::from(bytes);
        let parts = js_sys::Array::of1(&array);
        let options = web_sys::BlobPropertyBag::new();
        options.set_type(mime);
        let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
            .map_err(fail)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(fail)?;
        let document = web_sys::window()
            .and_then(|w| w.document())
            .ok_or("no document")?;
        let link: web_sys::HtmlAnchorElement =
            document.create_element("a").map_err(fail)?.unchecked_into();
        link.set_href(&url);
        link.set_download(name);
        // Some browsers only follow links that are on the page. The URL
        // isn't revoked: doing it right away can cancel the download.
        let body = document.body().ok_or("no page body")?;
        let _ = body.append_child(&link);
        // Click once the frame is done: the click passes through eframe's
        // own event handlers, which can't run while the app is updating.
        wasm_bindgen_futures::spawn_local(async move {
            link.click();
            link.remove();
        });
        Ok(())
    }

    /// Shows the browser's file picker; picked files arrive on `tx`.
    /// Called right after a click, which browsers require for pickers.
    pub fn pick(ctx: &egui::Context, tx: Sender<Picked>, song: bool) {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let Ok(input) = document.create_element("input") else {
            return;
        };
        let input: web_sys::HtmlInputElement = input.unchecked_into();
        input.set_type("file");
        let accept: Vec<String> = if song {
            vec![format!(".{}", crate::file::EXTENSION)]
        } else {
            crate::sample::EXTENSIONS
                .iter()
                .map(|e| format!(".{e}"))
                .collect()
        };
        input.set_accept(&accept.join(","));
        input.set_multiple(!song);
        let ctx = ctx.clone();
        let picked = input.clone();
        let on_change = Closure::<dyn FnMut()>::new(move || {
            let Some(files) = picked.files() else { return };
            for i in 0..files.length() {
                let Some(file) = files.get(i) else { continue };
                let (tx, ctx) = (tx.clone(), ctx.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    let Ok(buffer) = JsFuture::from(file.array_buffer()).await else {
                        return;
                    };
                    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                    let name = file.name();
                    let _ = tx.send(if song {
                        Picked::Song(name, bytes)
                    } else {
                        Picked::Sample(name, bytes)
                    });
                    ctx.request_repaint();
                });
            }
        });
        input.set_onchange(Some(on_change.as_ref().unchecked_ref()));
        // The input isn't on the page, so the handler can live as long as
        // the input does: forgetting it leaks one small closure per pick.
        on_change.forget();
        input.click();
    }

    /// Browsers only allow sound once the user has clicked or pressed a
    /// key, and only from inside that event's handler, so the audio stream
    /// starts on the first one. The handler keeps the stream alive.
    pub fn play_on_first_input(stream: cpal::Stream) {
        use cpal::traits::StreamTrait;
        use std::cell::Cell;

        let Some(window) = web_sys::window() else {
            return;
        };
        let started = Cell::new(false);
        let start = Closure::<dyn FnMut()>::new(move || {
            if !started.replace(true) {
                let _ = stream.play();
            }
        });
        for event in ["pointerdown", "keydown"] {
            let _ = window.add_event_listener_with_callback(event, start.as_ref().unchecked_ref());
        }
        start.forget();
    }

    pub fn set_title(title: &str) {
        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
            document.set_title(title);
        }
    }

    /// If the app crashes, says so on the page (and in the console) with
    /// the reason, instead of leaving a frozen picture of the app.
    pub fn show_crashes() {
        std::panic::set_hook(Box::new(|info| {
            let message = format!(
                "daw {} stopped working:\n\n{info}\n\nReload the page to start again.",
                env!("CARGO_PKG_VERSION")
            );
            web_sys::console::error_1(&message.clone().into());
            let status = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id("status"));
            if let Some(status) = status {
                status.set_text_content(Some(&message));
                let _ = status.set_attribute("class", "crashed");
            }
        }));
    }

    /// The canvas the app draws on, from `web/index.html`.
    pub fn canvas() -> web_sys::HtmlCanvasElement {
        web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("daw"))
            .expect("the page has a #daw canvas")
            .unchecked_into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        for len in 0..20 {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 + 200) as u8).collect();
            assert_eq!(base64_decode(&base64_encode(&bytes)).unwrap(), bytes);
        }
        assert_eq!(base64_decode("not*base64"), None);
    }
}
