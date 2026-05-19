use crate::constants::{BANNER_PATTERNS, DAMAGE_TYPES, JUKEBOX_SONGS};
use crate::protocol::{read_packet, write_packet, write_string, write_var_i32};
use crate::types::Config;
use anyhow::{Context, Result};
use tokio::net::TcpStream;

pub(super) async fn run_configuration(stream: &mut TcpStream, config: &Config) -> Result<()> {
    let _ = read_packet(stream).await;

    // Vanilla clients need these minimal configuration packets before play packets are valid.
    send_known_packs(stream, &config.version_name).await?;
    let _selected_packs = read_packet(stream).await?;
    send_required_registries(stream).await?;
    send_enabled_features(stream).await?;
    send_required_tags(stream).await?;
    send_finish_configuration(stream).await?;
    super::wait_for_packet_id(stream, 0x03, "finish configuration acknowledgement").await
}

async fn send_required_registries(stream: &mut TcpStream) -> Result<()> {
    // Send compact registries with just enough entries for a vanilla client to join.
    send_registry_data(stream, "minecraft:damage_type", DAMAGE_TYPES).await?;
    send_registry_data(stream, "minecraft:dimension_type", &["minecraft:overworld"]).await?;
    send_registry_data(stream, "minecraft:worldgen/biome", &["minecraft:plains"]).await?;
    send_registry_data(
        stream,
        "minecraft:timeline",
        &["minecraft:day", "minecraft:moon", "minecraft:early_game"],
    )
    .await?;
    send_registry_data(stream, "minecraft:world_clock", &["minecraft:overworld"]).await?;
    send_registry_data(
        stream,
        "minecraft:instrument",
        &[
            "minecraft:ponder_goat_horn",
            "minecraft:sing_goat_horn",
            "minecraft:seek_goat_horn",
            "minecraft:feel_goat_horn",
            "minecraft:admire_goat_horn",
            "minecraft:call_goat_horn",
            "minecraft:yearn_goat_horn",
            "minecraft:dream_goat_horn",
        ],
    )
    .await?;
    send_registry_data(stream, "minecraft:banner_pattern", BANNER_PATTERNS).await?;
    send_registry_data(stream, "minecraft:jukebox_song", JUKEBOX_SONGS).await?;
    send_registry_data(
        stream,
        "minecraft:cat_sound_variant",
        &["minecraft:classic", "minecraft:royal"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:cat_variant",
        &[
            "minecraft:tabby",
            "minecraft:black",
            "minecraft:red",
            "minecraft:siamese",
            "minecraft:british_shorthair",
            "minecraft:calico",
            "minecraft:persian",
            "minecraft:ragdoll",
            "minecraft:white",
            "minecraft:jellie",
            "minecraft:all_black",
        ],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:chicken_sound_variant",
        &["minecraft:classic", "minecraft:picky"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:chicken_variant",
        &["minecraft:temperate", "minecraft:warm", "minecraft:cold"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:cow_sound_variant",
        &["minecraft:classic", "minecraft:moody"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:cow_variant",
        &["minecraft:temperate", "minecraft:warm", "minecraft:cold"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:frog_variant",
        &["minecraft:temperate", "minecraft:warm", "minecraft:cold"],
    )
    .await?;
    send_registry_data(stream, "minecraft:painting_variant", &["minecraft:kebab"]).await?;
    send_registry_data(
        stream,
        "minecraft:pig_sound_variant",
        &["minecraft:classic", "minecraft:big", "minecraft:mini"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:pig_variant",
        &["minecraft:temperate", "minecraft:warm", "minecraft:cold"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:wolf_sound_variant",
        &[
            "minecraft:classic",
            "minecraft:big",
            "minecraft:cute",
            "minecraft:grumpy",
            "minecraft:puglin",
            "minecraft:sad",
            "minecraft:angry",
        ],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:wolf_variant",
        &[
            "minecraft:pale",
            "minecraft:spotted",
            "minecraft:snowy",
            "minecraft:black",
            "minecraft:ashen",
            "minecraft:rusty",
            "minecraft:woods",
            "minecraft:chestnut",
            "minecraft:striped",
        ],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:zombie_nautilus_variant",
        &["minecraft:temperate"],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:trim_material",
        &[
            "minecraft:quartz",
            "minecraft:iron",
            "minecraft:netherite",
            "minecraft:redstone",
            "minecraft:copper",
            "minecraft:gold",
            "minecraft:emerald",
            "minecraft:diamond",
            "minecraft:lapis",
            "minecraft:amethyst",
            "minecraft:resin",
        ],
    )
    .await?;
    send_registry_data(
        stream,
        "minecraft:trim_pattern",
        &[
            "minecraft:sentry",
            "minecraft:dune",
            "minecraft:coast",
            "minecraft:wild",
            "minecraft:ward",
            "minecraft:eye",
            "minecraft:vex",
            "minecraft:tide",
            "minecraft:snout",
            "minecraft:rib",
            "minecraft:spire",
            "minecraft:wayfinder",
            "minecraft:shaper",
            "minecraft:silence",
            "minecraft:raiser",
            "minecraft:host",
            "minecraft:flow",
            "minecraft:bolt",
        ],
    )
    .await
}

async fn send_registry_data(
    stream: &mut TcpStream,
    registry: &str,
    entries: &[&str],
) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, 0x07);
    write_string(&mut packet, registry)?;
    write_var_i32(&mut packet, entries.len() as i32);
    for entry in entries {
        write_string(&mut packet, entry)?;
        // False means the entry uses no inline NBT payload.
        packet.push(0);
    }
    write_packet(stream, &packet).await
}

async fn send_known_packs(stream: &mut TcpStream, version: &str) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, 0x0e);
    write_var_i32(&mut packet, 1);
    write_string(&mut packet, "minecraft")?;
    write_string(&mut packet, "core")?;
    write_string(&mut packet, version)?;
    write_packet(stream, &packet).await
}

async fn send_enabled_features(stream: &mut TcpStream) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, 0x0c);
    write_var_i32(&mut packet, 1);
    write_string(&mut packet, "minecraft:vanilla")?;
    write_packet(stream, &packet).await
}

async fn send_required_tags(stream: &mut TcpStream) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, 0x0d);
    write_var_i32(&mut packet, 3);
    write_string(&mut packet, "minecraft:damage_type")?;
    write_var_i32(&mut packet, 3);
    write_tag_ids(
        &mut packet,
        DAMAGE_TYPES,
        "minecraft:is_fire",
        &[
            "minecraft:in_fire",
            "minecraft:campfire",
            "minecraft:on_fire",
            "minecraft:lava",
            "minecraft:hot_floor",
            "minecraft:unattributed_fireball",
            "minecraft:fireball",
        ],
    )?;
    write_tag_ids(
        &mut packet,
        DAMAGE_TYPES,
        "minecraft:is_explosion",
        &[
            "minecraft:fireworks",
            "minecraft:explosion",
            "minecraft:player_explosion",
            "minecraft:bad_respawn_point",
        ],
    )?;
    write_tag_ids(
        &mut packet,
        DAMAGE_TYPES,
        "minecraft:bypasses_shield",
        &[
            "minecraft:on_fire",
            "minecraft:in_wall",
            "minecraft:cramming",
            "minecraft:drown",
            "minecraft:fly_into_wall",
            "minecraft:generic",
            "minecraft:wither",
            "minecraft:dragon_breath",
            "minecraft:starve",
            "minecraft:fall",
            "minecraft:ender_pearl",
            "minecraft:freeze",
            "minecraft:stalagmite",
            "minecraft:magic",
            "minecraft:indirect_magic",
            "minecraft:out_of_world",
            "minecraft:generic_kill",
            "minecraft:sonic_boom",
            "minecraft:outside_border",
            "minecraft:cactus",
            "minecraft:campfire",
            "minecraft:dry_out",
            "minecraft:falling_anvil",
            "minecraft:falling_stalactite",
            "minecraft:hot_floor",
            "minecraft:in_fire",
            "minecraft:lava",
            "minecraft:lightning_bolt",
            "minecraft:sweet_berry_bush",
        ],
    )?;

    write_string(&mut packet, "minecraft:timeline")?;
    write_var_i32(&mut packet, 1);
    write_string(&mut packet, "minecraft:in_overworld")?;
    write_var_i32(&mut packet, 3);
    write_var_i32(&mut packet, 0);
    write_var_i32(&mut packet, 1);
    write_var_i32(&mut packet, 2);

    write_string(&mut packet, "minecraft:banner_pattern")?;
    write_var_i32(&mut packet, 10);
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/flower",
        &["minecraft:flower"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/creeper",
        &["minecraft:creeper"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/skull",
        &["minecraft:skull"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/mojang",
        &["minecraft:mojang"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/globe",
        &["minecraft:globe"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/piglin",
        &["minecraft:piglin"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/flow",
        &["minecraft:flow"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/guster",
        &["minecraft:guster"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/field_masoned",
        &["minecraft:bricks"],
    )?;
    write_tag_ids(
        &mut packet,
        BANNER_PATTERNS,
        "minecraft:pattern_item/bordure_indented",
        &["minecraft:curly_border"],
    )?;
    write_packet(stream, &packet).await
}

fn write_tag_ids(out: &mut Vec<u8>, registry: &[&str], tag: &str, values: &[&str]) -> Result<()> {
    write_string(out, tag)?;
    write_var_i32(out, values.len() as i32);
    for value in values {
        let index = registry
            .iter()
            .position(|entry| entry == value)
            .with_context(|| format!("unknown registry entry {value} for tag {tag}"))?;
        write_var_i32(out, index as i32);
    }
    Ok(())
}

async fn send_finish_configuration(stream: &mut TcpStream) -> Result<()> {
    let mut packet = Vec::new();
    write_var_i32(&mut packet, 0x03);
    write_packet(stream, &packet).await
}
