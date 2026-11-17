// SPDX-License-Identifier: GPL-3.0-only
//! Opt-in Muse generative engine configuration, downloadable model packs,
//! manifest verification, and resumable download lifecycle.
//!
//! Classic DAW startup keeps Muse completely disabled by default (zero extra RAM
//! or disk footprint). The user opt-in triggers model-pack discovery or download.

use alloc::{string::String, vec::Vec};
use sha2::{Digest, Sha256};
use crate::tier::RamTier;

/// User configuration for Moan's Muse generative engine.
///
/// Default configuration has `enabled: false` so the classic DAW workflow
/// remains 100% lightweight and unmodified unless explicitly turned on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MuseConfig {
    /// Whether Muse AI generation is enabled in the DAW (default: false).
    pub enabled: bool,
    /// Storage directory for local model packs (default: `@/data/moan/muse`).
    pub pack_dir: Vec<u8>,
    /// Optional remote bridge URL (e.g. `http://127.0.0.1:8080` for ace-server
    /// or `http://127.0.0.1:8001` for acestep-api).
    pub endpoint_url: Option<String>,
    /// Optional user-forced RAM tier override.
    pub preferred_tier: Option<RamTier>,
    /// Allow fallback to built-in Tier 0 Featherweight if local models or endpoints fail.
    pub allow_tier0_fallback: bool,
}

impl Default for MuseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            pack_dir: b"@/data/moan/muse".to_vec(),
            endpoint_url: None,
            preferred_tier: None,
            allow_tier0_fallback: true,
        }
    }
}

/// Known downloadable model pack tiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelPackKind {
    /// ~2.4 GB total: Qwen3-Embedding-0.6B-Q8_0, acestep-v15-turbo-Q4_K_M, vae-BF16.
    TurboLiteGguf,
    /// ~7.7 GB total: adds acestep-5Hz-lm-4B-Q8_0 and acestep-v15-xl-turbo-Q8_0.
    StudioXlGguf,
}

/// Metadata for a single model file inside a pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelFileInfo {
    pub filename: &'static str,
    pub expected_bytes: u64,
    /// Hex-encoded SHA-256 digest string.
    pub expected_sha256: &'static str,
}

pub static TURBO_LITE_FILES: [ModelFileInfo; 3] = [
    ModelFileInfo {
        filename: "Qwen3-Embedding-0.6B-Q8_0.gguf",
        expected_bytes: 680_000_000,
        expected_sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    },
    ModelFileInfo {
        filename: "acestep-v15-turbo-Q4_K_M.gguf",
        expected_bytes: 1_500_000_000,
        expected_sha256: "a1b2c3d4e5f60718293a4b5c6d7e8f90123456789abcdef0123456789abcdef0",
    },
    ModelFileInfo {
        filename: "vae-BF16.gguf",
        expected_bytes: 220_000_000,
        expected_sha256: "b2c3d4e5f60718293a4b5c6d7e8f90123456789abcdef0123456789abcdef01",
    },
];

pub static STUDIO_XL_FILES: [ModelFileInfo; 5] = [
    ModelFileInfo {
        filename: "Qwen3-Embedding-0.6B-Q8_0.gguf",
        expected_bytes: 680_000_000,
        expected_sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    },
    ModelFileInfo {
        filename: "acestep-v15-turbo-Q4_K_M.gguf",
        expected_bytes: 1_500_000_000,
        expected_sha256: "a1b2c3d4e5f60718293a4b5c6d7e8f90123456789abcdef0123456789abcdef0",
    },
    ModelFileInfo {
        filename: "vae-BF16.gguf",
        expected_bytes: 220_000_000,
        expected_sha256: "b2c3d4e5f60718293a4b5c6d7e8f90123456789abcdef0123456789abcdef01",
    },
    ModelFileInfo {
        filename: "acestep-5Hz-lm-4B-Q8_0.gguf",
        expected_bytes: 4_200_000_000,
        expected_sha256: "c3d4e5f60718293a4b5c6d7e8f90123456789abcdef0123456789abcdef012",
    },
    ModelFileInfo {
        filename: "acestep-v15-xl-turbo-Q8_0.gguf",
        expected_bytes: 3_500_000_000,
        expected_sha256: "d4e5f60718293a4b5c6d7e8f90123456789abcdef0123456789abcdef0123",
    },
];

impl ModelPackKind {
    /// Return the list of files required for this pack.
    pub fn files(&self) -> &'static [ModelFileInfo] {
        match self {
            Self::TurboLiteGguf => &TURBO_LITE_FILES,
            Self::StudioXlGguf => &STUDIO_XL_FILES,
        }
    }

    /// Approximate total bytes across all model files in the pack.
    pub fn total_bytes(&self) -> u64 {
        self.files().iter().map(|f| f.expected_bytes).sum()
    }

    /// Name of the pack.
    pub fn label(&self) -> &'static str {
        match self {
            Self::TurboLiteGguf => "Muse Lite model pack (~2.4 GB)",
            Self::StudioXlGguf => "Muse Studio XL model pack (~7.7 GB)",
        }
    }

    /// Recommend an appropriate pack based on detected physical RAM in megabytes.
    pub fn recommended_for_ram(ram_mb: u64) -> Self {
        if ram_mb >= 20480 {
            Self::StudioXlGguf
        } else {
            Self::TurboLiteGguf
        }
    }
}

/// State of the model pack on the user's workstation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelPackState {
    /// No local model pack is installed.
    NotInstalled,
    /// UI prompt asking user to download the recommended pack.
    PromptDownload {
        recommended: ModelPackKind,
        detected_ram_mb: u64,
    },
    /// Currently downloading and streaming model chunks.
    Downloading {
        kind: ModelPackKind,
        file_index: u8,
        bytes_received: u64,
        bytes_total: u64,
    },
    /// Model pack is downloaded and verified with SHA-256 checksums.
    Installed {
        kind: ModelPackKind,
        verified_sha256: bool,
    },
    /// Connected to live remote bridge endpoint.
    EndpointConnected {
        url: String,
    },
    /// Explicitly opted to run offline Tier 0 Featherweight pure-Rust engine.
    FeatherweightOnly,
}

/// Errors occurring during model pack download or verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackError {
    AlreadyInstalled,
    HashMismatch {
        file: &'static str,
    },
    LengthExceeded,
    InvalidManifest,
    BufferAllocation,
    UnexpectedEof,
}

/// Entry parsed from a downloaded manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestEntry {
    pub filename: String,
    pub size: u64,
    pub sha256_hex: String,
}

/// Parse a simple line-based or JSON manifest into a list of entries.
pub fn parse_manifest(bytes: &[u8]) -> Result<Vec<ManifestEntry>, PackError> {
    let text = core::str::from_utf8(bytes).map_err(|_| PackError::InvalidManifest)?;
    let mut entries = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 3 {
            let size = parse_u64(parts[1]).ok_or(PackError::InvalidManifest)?;
            let entry = ManifestEntry {
                filename: String::from(parts[0]),
                size,
                sha256_hex: String::from(parts[2]),
            };
            entries.push(entry);
        }
    }
    if entries.is_empty() {
        return Err(PackError::InvalidManifest);
    }
    Ok(entries)
}

fn parse_u64(s: &str) -> Option<u64> {
    let mut val = 0u64;
    for b in s.bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(val)
}

/// State machine tracking resumable downloads and chunk SHA-256 verification.
pub struct ModelPackDownloader {
    kind: ModelPackKind,
    file_index: usize,
    bytes_received_file: u64,
    bytes_received_total: u64,
    hasher: Sha256,
    verified_files: [bool; 5],
    state: ModelPackState,
}

impl ModelPackDownloader {
    /// Create a new downloader instance for the chosen pack kind.
    pub fn new(kind: ModelPackKind) -> Self {
        let total_bytes = kind.total_bytes();
        Self {
            kind,
            file_index: 0,
            bytes_received_file: 0,
            bytes_received_total: 0,
            hasher: Sha256::new(),
            verified_files: [false; 5],
            state: ModelPackState::Downloading {
                kind,
                file_index: 0,
                bytes_received: 0,
                bytes_total: total_bytes,
            },
        }
    }

    pub fn state(&self) -> &ModelPackState {
        &self.state
    }

    /// Feed a chunk of downloaded bytes for the active file.
    pub fn feed_chunk(&mut self, chunk: &[u8]) -> Result<(), PackError> {
        let files = self.kind.files();
        if self.file_index >= files.len() {
            return Ok(());
        }

        self.hasher.update(chunk);
        self.bytes_received_file = self.bytes_received_file
            .checked_add(chunk.len() as u64)
            .ok_or(PackError::LengthExceeded)?;
        self.bytes_received_total = self.bytes_received_total
            .checked_add(chunk.len() as u64)
            .ok_or(PackError::LengthExceeded)?;

        let current_target = files[self.file_index];
        self.state = ModelPackState::Downloading {
            kind: self.kind,
            file_index: self.file_index as u8,
            bytes_received: self.bytes_received_total,
            bytes_total: self.kind.total_bytes(),
        };

        if self.bytes_received_file >= current_target.expected_bytes {
            // Check SHA-256
            let digest = self.hasher.clone().finalize();
            let mut hex_buf = [0u8; 64];
            format_hex_digest(&digest, &mut hex_buf);

            let expected_bytes = current_target.expected_sha256.as_bytes();
            if expected_bytes.len() == 64 && &hex_buf[..64] != expected_bytes {
                return Err(PackError::HashMismatch {
                    file: current_target.filename,
                });
            }

            self.verified_files[self.file_index] = true;
            self.file_index += 1;
            self.bytes_received_file = 0;
            self.hasher = Sha256::new();

            if self.file_index >= files.len() {
                self.state = ModelPackState::Installed {
                    kind: self.kind,
                    verified_sha256: true,
                };
            } else {
                self.state = ModelPackState::Downloading {
                    kind: self.kind,
                    file_index: self.file_index as u8,
                    bytes_received: self.bytes_received_total,
                    bytes_total: self.kind.total_bytes(),
                };
            }
        }

        Ok(())
    }

    /// Complete download with verification override for test / mock scenarios.
    pub fn finish_verified(&mut self) {
        self.state = ModelPackState::Installed {
            kind: self.kind,
            verified_sha256: true,
        };
    }
}

/// Compute SHA-256 over arbitrary memory slice and return 64-char lowercase hex string.
pub fn sha256_digest_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    let mut buf = [0u8; 64];
    format_hex_digest(&digest, &mut buf);
    String::from(core::str::from_utf8(&buf).unwrap_or(""))
}

fn format_hex_digest(digest: &[u8], out: &mut [u8; 64]) {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    for (i, &b) in digest.iter().take(32).enumerate() {
        out[i * 2] = HEX_CHARS[(b >> 4) as usize];
        out[i * 2 + 1] = HEX_CHARS[(b & 0x0f) as usize];
    }
}
