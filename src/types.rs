use crate::constants::{
    DEFAULT_ADDR, DEFAULT_PROTOCOL, DEFAULT_VERSION_NAME, PLAYER_INVENTORY_SLOT_COUNT, SPAWN_Y,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::{LazyLock, atomic::AtomicI32};
use tokio::sync::{Mutex, mpsc};

pub(crate) static NEXT_ENTITY_ID: AtomicI32 = AtomicI32::new(2);

// Runtime state is tiny for now, so process-local locks keep the server simple.
pub(crate) static ONLINE_PLAYERS: LazyLock<Mutex<Vec<OnlinePlayer>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));
pub(crate) static WORLD_BLOCKS: LazyLock<Mutex<HashMap<(i32, i32, i32), i32>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// Item-to-block placement data comes from the generated CSV at startup.
pub(crate) static BLOCK_ITEM_PLACEMENTS: LazyLock<HashMap<i32, BlockPlacement>> =
    LazyLock::new(crate::server::load_block_item_placements);

#[derive(Clone, Copy)]
pub(crate) enum BlockPlacementKind {
    Single,
    Double,
    Bed,
}

#[derive(Clone, Copy)]
pub(crate) struct BlockPlacement {
    pub(crate) block_name: &'static str,
    pub(crate) lower_state_id: i32,
    pub(crate) upper_state_id: i32,
    pub(crate) kind: BlockPlacementKind,
}

#[derive(Clone)]
pub(crate) struct ProfileProperty {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) signature: Option<String>,
}

#[derive(Clone)]
pub(crate) struct OnlinePlayer {
    pub(crate) entity_id: i32,
    pub(crate) uuid: [u8; 16],
    pub(crate) username: String,
    pub(crate) profile_properties: Vec<ProfileProperty>,
    pub(crate) sender: mpsc::UnboundedSender<Vec<u8>>,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: f64,
    pub(crate) y_rot: f32,
    pub(crate) x_rot: f32,
    pub(crate) on_ground: bool,
    pub(crate) loaded_chunks: HashSet<(i32, i32)>,
    pub(crate) held_slot: i16,
    pub(crate) inventory_slots: Vec<Option<PersistedInventoryItem>>,
    pub(crate) game_mode: GameMode,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum GameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
}

impl Default for GameMode {
    fn default() -> Self {
        Self::Creative
    }
}

impl GameMode {
    pub(crate) fn id(self) -> i32 {
        match self {
            Self::Survival => 0,
            Self::Creative => 1,
            Self::Adventure => 2,
            Self::Spectator => 3,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Survival => "survival",
            Self::Creative => "creative",
            Self::Adventure => "adventure",
            Self::Spectator => "spectator",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "0" | "s" | "survival" => Some(Self::Survival),
            "1" | "c" | "creative" => Some(Self::Creative),
            "2" | "a" | "adventure" => Some(Self::Adventure),
            "3" | "sp" | "spectator" => Some(Self::Spectator),
            _ => None,
        }
    }

    pub(crate) fn from_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(Self::Survival),
            1 => Some(Self::Creative),
            2 => Some(Self::Adventure),
            3 => Some(Self::Spectator),
            _ => None,
        }
    }

    pub(crate) fn allows_building(self) -> bool {
        matches!(self, Self::Survival | Self::Creative)
    }

    pub(crate) fn ability_flags(self) -> u8 {
        match self {
            // Creative grants invulnerability, flight permission, and instant build.
            Self::Creative => 0x01 | 0x04 | 0x08,
            Self::Spectator => 0x01 | 0x02 | 0x04,
            Self::Survival | Self::Adventure => 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PersistedInventoryItem {
    pub(crate) item_id: i32,
    pub(crate) count: i32,
    #[serde(default)]
    pub(crate) encoded: Vec<u8>,
}

pub(crate) fn empty_inventory_slots() -> Vec<Option<PersistedInventoryItem>> {
    vec![None; PLAYER_INVENTORY_SLOT_COUNT]
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PersistedPlayerData {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: f64,
    pub(crate) y_rot: f32,
    pub(crate) x_rot: f32,
    pub(crate) on_ground: bool,
    pub(crate) held_slot: i16,
    #[serde(default = "empty_inventory_slots")]
    pub(crate) inventory_slots: Vec<Option<PersistedInventoryItem>>,
    #[serde(default)]
    pub(crate) game_mode: GameMode,
}

impl Default for PersistedPlayerData {
    fn default() -> Self {
        Self {
            x: 0.5,
            y: SPAWN_Y,
            z: 0.5,
            y_rot: 0.0,
            x_rot: 0.0,
            on_ground: true,
            held_slot: 0,
            inventory_slots: empty_inventory_slots(),
            game_mode: GameMode::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Config {
    pub(crate) addr: String,
    pub(crate) version_name: String,
    pub(crate) protocol: i32,
    pub(crate) motd: String,
    pub(crate) max_players: i32,
}

impl Config {
    pub(crate) fn from_env() -> Self {
        Self {
            addr: env::var("VIBECRAFT_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string()),
            version_name: env::var("MINECRAFT_VERSION")
                .unwrap_or_else(|_| DEFAULT_VERSION_NAME.to_string()),
            protocol: env::var("MINECRAFT_PROTOCOL")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(DEFAULT_PROTOCOL),
            motd: env::var("VIBECRAFT_MOTD")
                .unwrap_or_else(|_| "VibeCraft: Rust-powered Minecraft server".to_string()),
            max_players: env::var("VIBECRAFT_MAX_PLAYERS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(20),
        }
    }
}
