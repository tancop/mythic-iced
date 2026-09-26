use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use iced::Task;
use rkyv::{Archive, Deserialize, Serialize};

use crate::{Message, State};

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
pub struct ImageLibraryData {
    pub images: HashMap<String, Vec<u8>>,
}

pub struct ImageLibrary {
    live: HashMap<String, Vec<u8>>,
}

impl ImageLibrary {
    pub fn load(path: &PathBuf) -> Self {
        match fs::read(path) {
            Ok(bytes) => {
                let archived = unsafe {
                    rkyv::access_unchecked::<<ImageLibraryData as Archive>::Archived>(&bytes)
                };
                let mut live = HashMap::new();
                for entry in archived.images.iter() {
                    live.insert(entry.0.to_string(), entry.1.to_vec());
                }
                log::info!("Loaded image library: {} images", live.len());
                Self { live }
            }
            Err(_) => Self::empty(),
        }
    }

    pub fn empty() -> Self {
        Self {
            live: HashMap::new(),
        }
    }

    pub fn get(&self, id: &str) -> Option<&[u8]> {
        self.live.get(id).map(|v| v.as_slice())
    }

    pub fn insert(&mut self, id: String, bytes: Vec<u8>) {
        self.live.insert(id, bytes);
    }

    pub fn save(&self, path: &PathBuf) -> anyhow::Result<()> {
        let data = ImageLibraryData {
            images: self.live.clone(),
        };
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&data)?;
        fs::write(path, &*bytes)?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct DecodedCard {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<Vec<u8>>,
}

pub struct PixelData {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<Vec<u8>>,
}

pub fn library_path() -> PathBuf {
    dirs::cache_dir().unwrap().join("mythic").join("images.db")
}

const THUMB_WIDTH: u32 = 255;
const THUMB_HEIGHT: u32 = 340;

pub fn resize_image(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let img = image::load_from_memory(bytes)?;
    let resized = img.resize(
        THUMB_WIDTH,
        THUMB_HEIGHT,
        image::imageops::FilterType::Lanczos3,
    );
    let mut buf = std::io::Cursor::new(Vec::new());
    resized.write_to(&mut buf, image::ImageFormat::Jpeg)?;
    Ok(buf.into_inner())
}

pub fn handle_downloaded(state: &mut State, id: String, bytes: Arc<Vec<u8>>) -> Task<Message> {
    state.image_library.insert(id, (*bytes).clone());
    let _ = state.image_library.save(&library_path());
    decode_visible(state)
}

pub fn handle_chunk_decoded(
    state: &mut State,
    decoded: Vec<DecodedCard>,
    failed: Vec<String>,
) -> Task<Message> {
    for id in failed {
        state.inflight_decodes.remove(&id);
    }
    for card in decoded {
        state.inflight_decodes.remove(&card.id);
        state.decoded_images.insert(
            card.id,
            PixelData {
                width: card.width,
                height: card.height,
                pixels: card.pixels,
            },
        );
    }
    Task::none()
}

pub fn decode_visible(state: &mut State) -> Task<Message> {
    let Some(items) = &state.library_items else {
        return Task::none();
    };

    let cols = crate::ui::library::cols_for_width(state.viewport_width);
    let (lo, hi) = crate::library::visible_range(state);

    // One task per grid chunk so a whole row swaps in atomically instead of
    // cards popping in one by one (each completion re-renders the grid).
    let mut chunks: Vec<Vec<(String, Vec<u8>)>> = Vec::new();

    for chunk in items.chunks(cols).skip(lo).take(hi.saturating_sub(lo)) {
        let mut pending: Vec<(String, Vec<u8>)> = Vec::new();
        for item in chunk {
            let Some(catalog) = state.catalog_items.get(item.catalog_item_id.as_ref()) else {
                continue;
            };
            if state.decoded_images.contains_key(&catalog.id)
                || !state.inflight_decodes.insert(catalog.id.clone())
            {
                continue;
            }
            match state.image_library.get(&catalog.id) {
                Some(bytes) if !bytes.is_empty() => {
                    pending.push((catalog.id.clone(), bytes.to_vec()));
                }
                _ => {
                    // Not cached yet (or cached empty): don't wedge, retry on
                    // the next scroll event.
                    state.inflight_decodes.remove(&catalog.id);
                }
            }
        }
        if !pending.is_empty() {
            chunks.push(pending);
        }
    }

    Task::batch(
        chunks
            .into_iter()
            .map(|chunk| Task::future(async move { decode_chunk(chunk).await })),
    )
}

async fn decode_chunk(chunk: Vec<(String, Vec<u8>)>) -> Message {
    let mut decoded = Vec::with_capacity(chunk.len());
    let mut failed = Vec::new();
    for (id, bytes) in chunk {
        match image::load_from_memory(&bytes) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (width, height) = rgba.dimensions();
                decoded.push(DecodedCard {
                    id,
                    width,
                    height,
                    pixels: Arc::new(rgba.into_raw()),
                });
            }
            Err(e) => {
                log::error!("Failed to decode image {}: {}", id, e);
                failed.push(id);
            }
        }
    }
    Message::ChunkDecoded { decoded, failed }
}
