// SPDX-License-Identifier: GPL-3.0-or-later
//
// Memory-card backing: each (port, slot) is an 8MB file in a sandboxed save
// directory the host owns. Byte-range read/write/flush over the C ABI. The card
// image is created and zero-filled on first access so guest formatting writes
// land in a real file.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

const MC_SIZE: u64 = 8 * 1024 * 1024; // 8MB PS2 memory card
const MC_PAGE_SIZE: u32 = 0x400; // 1KB page
const MC_PAGES_PER_BLOCK: u32 = 16; // 16KB cluster
const MC_TOTAL_BLOCKS: u32 = (MC_SIZE / (MC_PAGE_SIZE as u64 * MC_PAGES_PER_BLOCK as u64)) as u32;

pub struct SaveCards {
    dir: PathBuf,
    // Lazily opened, indexed [port][slot]; PS2 has 2 ports, 1 slot (no multitap).
    files: [[Option<File>; 1]; 2],
}

impl SaveCards {
    pub fn new() -> SaveCards {
        SaveCards {
            dir: default_save_dir(),
            files: Default::default(),
        }
    }

    fn valid(port: i32, slot: i32) -> bool {
        (0..2).contains(&port) && slot == 0
    }

    /// Open (creating + sizing if needed) the card file for port/slot.
    fn card(&mut self, port: i32, slot: i32) -> Option<&mut File> {
        if !Self::valid(port, slot) {
            return None;
        }
        let p = port as usize;
        let s = slot as usize;
        if self.files[p][s].is_none() {
            if std::fs::create_dir_all(&self.dir).is_err() {
                return None;
            }
            let path = self.dir.join(format!("mc{}-{}.ps2", port, slot));
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false) // preserve existing card contents
                .open(&path)
                .ok()?;
            // Ensure the backing is exactly card-sized.
            if let Ok(meta) = file.metadata() {
                if meta.len() != MC_SIZE {
                    file.set_len(MC_SIZE).ok()?;
                }
            }
            self.files[p][s] = Some(file);
        }
        self.files[p][s].as_mut()
    }

    pub fn read(&mut self, port: i32, slot: i32, offset: u64, dst: &mut [u8]) -> bool {
        if offset + dst.len() as u64 > MC_SIZE {
            return false;
        }
        let Some(file) = self.card(port, slot) else {
            return false;
        };
        if file.seek(SeekFrom::Start(offset)).is_err() {
            return false;
        }
        file.read_exact(dst).is_ok()
    }

    pub fn write(&mut self, port: i32, slot: i32, offset: u64, src: &[u8]) -> bool {
        if offset + src.len() as u64 > MC_SIZE {
            return false;
        }
        let Some(file) = self.card(port, slot) else {
            return false;
        };
        if file.seek(SeekFrom::Start(offset)).is_err() {
            return false;
        }
        file.write_all(src).is_ok()
    }

    pub fn flush(&mut self, port: i32, slot: i32) -> bool {
        let Some(file) = self.card(port, slot) else {
            return false;
        };
        file.flush().is_ok() && file.sync_all().is_ok()
    }

    /// Report card metadata: type (2 = PS2 card), free blocks, format flag (1 =
    /// formatted). We treat the file as present and formatted; free-block
    /// accounting is reported as the full card since we do not parse the FAT.
    pub fn info(&mut self, port: i32, slot: i32) -> Option<(u32, u32, u32)> {
        self.card(port, slot)?;
        Some((2, MC_TOTAL_BLOCKS, 1))
    }
}

fn default_save_dir() -> PathBuf {
    // Honor an explicit override first; otherwise a per-user data dir.
    if let Ok(dir) = std::env::var("PS2X_SAVE_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("ps2recomp")
            .join("memcards");
    }
    PathBuf::from("memcards")
}
