//! Saved games: a universe's [`Snapshot`] as bytes, under a line that says what it is.
//!
//! A save is one line of JSON (the [`Header`], for listing saves without reading them) and then
//! the snapshot in MessagePack with its fields named. MessagePack because it keeps every number
//! bit for bit, infinities included, which JSON cannot, and because what comes from the mods'
//! records (tagged enums, flattened fields) reads back just as it does from a record.
//!
//! A vessel in flight is saved whole, parts and all, not as the craft it was built from: it has
//! lost stages and burned fuel, and a craft file changed since must not change it. So loading
//! needs no records but the solar system's, and a mod can change or go without touching what
//! is already flying. The header names the mods the game was played with, so that what the game
//! draws and what the editor offers can follow it.

use std::{
    collections::BTreeSet,
    hash::{DefaultHasher, Hash, Hasher},
    sync::{LazyLock, Mutex},
};

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    time::SimTime,
    universe::{Snapshot, Universe},
};

/// Bump this when a save would no longer read back as it was meant.
pub const FORMAT: u32 = 1;

/// What the list of saves says about a save.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    pub format: u32,
    pub name: String,
    /// When it was saved, seconds since the Unix epoch.
    pub saved_at: u64,
    /// The game's clock.
    pub time: SimTime,
    /// The vessel being flown.
    pub flying: String,
    /// How many vessels are in the game, debris left out.
    pub vessels: usize,
    /// The mods the game is played with, the base game aside; `None` in a save from before saves
    /// said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mods: Option<Vec<String>>,
}

impl Header {
    /// `name`'s header for `universe` as it is now.
    pub fn new(name: &str, saved_at: u64, universe: &Universe) -> Self {
        let flying = universe.focus.and_then(|id| universe.vessel(id));
        Self {
            format: FORMAT,
            name: name.to_owned(),
            saved_at,
            time: universe.time(),
            flying: flying.map_or(String::new(), |v| v.name.clone()),
            vessels: universe.vessels.iter().filter(|v| !v.is_debris()).count(),
            mods: None,
        }
    }
}

/// The bytes of a save of `universe`.
pub fn write(header: &Header, universe: &Universe) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(header).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    let body = rmp_serde::to_vec_named(&universe.snapshot()).map_err(|e| e.to_string())?;
    bytes.extend(body);
    Ok(bytes)
}

/// What a save of `universe` would hold, as one number: two that are equal hold the same game, to
/// tell whether leaving now would lose anything. Comparing the clock would not do, since a vessel can
/// be destroyed with the game paused.
pub fn fingerprint(universe: &Universe) -> Result<u64, String> {
    let body = rmp_serde::to_vec_named(&universe.snapshot()).map_err(|e| e.to_string())?;
    let mut hasher = DefaultHasher::new();
    body.hash(&mut hasher);
    Ok(hasher.finish())
}

/// What a save says it is, from its first line (all of a save may be given, or just the start).
pub fn read_header(bytes: &[u8]) -> Result<Header, String> {
    let end = bytes
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(bytes.len());
    serde_json::from_slice(&bytes[..end]).map_err(|e| format!("not a save: {e}"))
}

/// The snapshot in a save, if this version of the game can read it.
pub fn read(bytes: &[u8]) -> Result<Snapshot, String> {
    let header = read_header(bytes)?;
    if header.format != FORMAT {
        return Err(format!(
            "it was saved by another version of the game (format {}, this one reads {FORMAT})",
            header.format
        ));
    }
    let start = bytes
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |n| n + 1);
    rmp_serde::from_slice(&bytes[start..]).map_err(|e| format!("it cannot be read: {e}"))
}

/// Text the game keeps for good, like the name of an action. Under this name a field of it can
/// be read with [`interned`]: serde would have a plain reference borrow from what it reads.
pub(crate) type Kept = &'static str;

/// Read a [`Kept`] text. Each different one is kept once, however many saves are loaded.
pub(crate) fn interned<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Kept, D::Error> {
    static TEXTS: LazyLock<Mutex<BTreeSet<Kept>>> = LazyLock::new(Mutex::default);
    let text = String::deserialize(deserializer)?;
    let mut texts = TEXTS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(known) = texts.get(text.as_str()) {
        return Ok(known);
    }
    let kept: Kept = Box::leak(text.into_boxed_str());
    texts.insert(kept);
    Ok(kept)
}
