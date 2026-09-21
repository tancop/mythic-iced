use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use rkyv::{Archive, Deserialize, Serialize};

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
