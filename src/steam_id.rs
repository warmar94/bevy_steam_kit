//! SteamID helpers shared by every feature.

/// Is `id` an individual (user) SteamID64 in the public universe?
/// Universe `id >> 56 == 1`, account type `(id >> 52) & 0xF == 1`, instance
/// `(id >> 32) & 0xFFFFF == 1`, account id `id as u32 != 0`.
pub fn is_individual_steam_id64(id: u64) -> bool {
    let universe = id >> 56;
    let account_type = (id >> 52) & 0xF;
    let instance = (id >> 32) & 0xF_FFFF;
    let account = id as u32;
    universe == 1 && account_type == 1 && instance == 1 && account != 0
}
