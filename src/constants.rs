pub(crate) const DEFAULT_ADDR: &str = "0.0.0.0:25565";
pub(crate) const DEFAULT_VERSION_NAME: &str = "26.1.2";
pub(crate) const DEFAULT_PROTOCOL: i32 = 775;

// Packet and registry IDs are tied to the protocol version above.
pub(crate) const DIMENSION: &str = "minecraft:overworld";
pub(crate) const VIEW_DISTANCE: i32 = 6;
pub(crate) const SPAWN_Y: f64 = 65.0;
pub(crate) const CHUNK_BATCH_FINISHED_PACKET_ID: i32 = 0x0b;
pub(crate) const CHUNK_BATCH_START_PACKET_ID: i32 = 0x0c;
pub(crate) const PLAY_ADD_ENTITY_PACKET_ID: i32 = 0x01;
pub(crate) const PLAY_BLOCK_UPDATE_PACKET_ID: i32 = 0x08;
pub(crate) const PLAY_CONTAINER_SET_SLOT_PACKET_ID: i32 = 0x14;
pub(crate) const PLAY_LEVEL_CHUNK_WITH_LIGHT_PACKET_ID: i32 = 0x2d;
pub(crate) const PLAY_LOGIN_PACKET_ID: i32 = 0x31;
pub(crate) const PLAY_GAME_EVENT_PACKET_ID: i32 = 0x26;
pub(crate) const PLAY_KEEP_ALIVE_PACKET_ID: i32 = 0x2c;
pub(crate) const PLAY_PLAYER_INFO_REMOVE_PACKET_ID: i32 = 0x45;
pub(crate) const PLAY_PLAYER_INFO_UPDATE_PACKET_ID: i32 = 0x46;
pub(crate) const PLAY_PLAYER_POSITION_PACKET_ID: i32 = 0x48;
pub(crate) const PLAY_REMOVE_ENTITIES_PACKET_ID: i32 = 0x4d;
pub(crate) const PLAY_ROTATE_HEAD_PACKET_ID: i32 = 0x53;
pub(crate) const PLAY_SET_CHUNK_CACHE_CENTER_PACKET_ID: i32 = 0x5e;
pub(crate) const PLAY_SET_CHUNK_CACHE_RADIUS_PACKET_ID: i32 = 0x5f;
pub(crate) const PLAY_SET_DEFAULT_SPAWN_POSITION_PACKET_ID: i32 = 0x61;
pub(crate) const PLAY_SET_ENTITY_DATA_PACKET_ID: i32 = 0x63;
pub(crate) const PLAY_SET_HELD_SLOT_PACKET_ID: i32 = 0x69;
pub(crate) const PLAY_ENTITY_POSITION_SYNC_PACKET_ID: i32 = 0x23;
pub(crate) const PLAYER_ENTITY_TYPE_ID: i32 = 155;
pub(crate) const PLAYER_HOTBAR_SLOT_START: usize = 36;
pub(crate) const PLAYER_OFFHAND_SLOT: usize = 45;
pub(crate) const PLAYER_INVENTORY_SLOT_COUNT: usize = 46;
pub(crate) const SERVERBOUND_MOVE_PLAYER_POS_PACKET_ID: i32 = 0x1e;
pub(crate) const SERVERBOUND_MOVE_PLAYER_POS_ROT_PACKET_ID: i32 = 0x1f;
pub(crate) const SERVERBOUND_MOVE_PLAYER_ROT_PACKET_ID: i32 = 0x20;
pub(crate) const SERVERBOUND_MOVE_PLAYER_STATUS_ONLY_PACKET_ID: i32 = 0x21;
pub(crate) const SERVERBOUND_SET_CARRIED_ITEM_PACKET_ID: i32 = 0x35;
pub(crate) const SERVERBOUND_PLAYER_ACTION_PACKET_ID: i32 = 0x29;
pub(crate) const SERVERBOUND_SET_CREATIVE_MODE_SLOT_PACKET_ID: i32 = 0x38;
pub(crate) const SERVERBOUND_USE_ITEM_ON_PACKET_ID: i32 = 0x42;
pub(crate) const BLOCK_STATE_GRASS_BLOCK: i32 = 9;
pub(crate) const BLOCK_STATE_DIRT: i32 = 10;
pub(crate) const BLOCK_STATE_BEDROCK: i32 = 85;
pub(crate) const BLOCK_STATE_OAK_LOG: i32 = 137;
pub(crate) const BLOCK_STATE_OAK_LEAVES: i32 = 279;

pub(crate) const DAMAGE_TYPES: &[&str] = &[
    "minecraft:arrow",
    "minecraft:bad_respawn_point",
    "minecraft:cactus",
    "minecraft:campfire",
    "minecraft:cramming",
    "minecraft:dragon_breath",
    "minecraft:drown",
    "minecraft:dry_out",
    "minecraft:ender_pearl",
    "minecraft:explosion",
    "minecraft:fall",
    "minecraft:falling_anvil",
    "minecraft:falling_block",
    "minecraft:falling_stalactite",
    "minecraft:fireball",
    "minecraft:fireworks",
    "minecraft:fly_into_wall",
    "minecraft:freeze",
    "minecraft:generic",
    "minecraft:generic_kill",
    "minecraft:hot_floor",
    "minecraft:in_fire",
    "minecraft:in_wall",
    "minecraft:indirect_magic",
    "minecraft:lava",
    "minecraft:lightning_bolt",
    "minecraft:mace_smash",
    "minecraft:magic",
    "minecraft:mob_attack",
    "minecraft:mob_attack_no_aggro",
    "minecraft:mob_projectile",
    "minecraft:on_fire",
    "minecraft:out_of_world",
    "minecraft:outside_border",
    "minecraft:player_attack",
    "minecraft:player_explosion",
    "minecraft:sonic_boom",
    "minecraft:spear",
    "minecraft:spit",
    "minecraft:stalagmite",
    "minecraft:starve",
    "minecraft:sting",
    "minecraft:sweet_berry_bush",
    "minecraft:thorns",
    "minecraft:thrown",
    "minecraft:trident",
    "minecraft:unattributed_fireball",
    "minecraft:wind_charge",
    "minecraft:wither",
    "minecraft:wither_skull",
];

pub(crate) const BANNER_PATTERNS: &[&str] = &[
    "minecraft:base",
    "minecraft:square_bottom_left",
    "minecraft:square_bottom_right",
    "minecraft:square_top_left",
    "minecraft:square_top_right",
    "minecraft:stripe_bottom",
    "minecraft:stripe_top",
    "minecraft:stripe_left",
    "minecraft:stripe_right",
    "minecraft:stripe_center",
    "minecraft:stripe_middle",
    "minecraft:stripe_downright",
    "minecraft:stripe_downleft",
    "minecraft:small_stripes",
    "minecraft:cross",
    "minecraft:straight_cross",
    "minecraft:triangle_bottom",
    "minecraft:triangle_top",
    "minecraft:triangles_bottom",
    "minecraft:triangles_top",
    "minecraft:diagonal_left",
    "minecraft:diagonal_up_right",
    "minecraft:diagonal_up_left",
    "minecraft:diagonal_right",
    "minecraft:circle",
    "minecraft:rhombus",
    "minecraft:half_vertical",
    "minecraft:half_horizontal",
    "minecraft:half_vertical_right",
    "minecraft:half_horizontal_bottom",
    "minecraft:border",
    "minecraft:curly_border",
    "minecraft:gradient",
    "minecraft:gradient_up",
    "minecraft:bricks",
    "minecraft:globe",
    "minecraft:creeper",
    "minecraft:skull",
    "minecraft:flower",
    "minecraft:mojang",
    "minecraft:piglin",
    "minecraft:flow",
    "minecraft:guster",
];

pub(crate) const JUKEBOX_SONGS: &[&str] = &[
    "minecraft:13",
    "minecraft:cat",
    "minecraft:blocks",
    "minecraft:chirp",
    "minecraft:far",
    "minecraft:mall",
    "minecraft:mellohi",
    "minecraft:stal",
    "minecraft:strad",
    "minecraft:ward",
    "minecraft:11",
    "minecraft:wait",
    "minecraft:otherside",
    "minecraft:5",
    "minecraft:pigstep",
    "minecraft:relic",
    "minecraft:precipice",
    "minecraft:tears",
    "minecraft:lava_chicken",
    "minecraft:creator",
    "minecraft:creator_music_box",
];
