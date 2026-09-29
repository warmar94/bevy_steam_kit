//! Name checks shared by the `ISteamUserStats` features (`stats`, `leaderboards`).
//!
//! steamworks 0.12.2 turns every stat, achievement and leaderboard name into a C string with
//! `CString::new(name).unwrap()`, which PANICS on an interior NUL byte. The kit checks every name
//! with these functions before any Steam call.

/// The longest stat / achievement API name, in bytes (`k_cchStatNameMax` is 128 including the
/// terminating NUL).
pub const MAX_API_NAME_BYTES: usize = 127;

/// The longest leaderboard name the kit accepts, in bytes. The SDK's `k_cchLeaderboardNameMax`
/// is 128; whether that counts the terminating NUL is not documented, so the kit uses the safe
/// 127, the same as for stat and achievement names.
pub const MAX_LEADERBOARD_NAME_BYTES: usize = 127;

fn valid(name: &str, max: usize) -> bool {
    !name.is_empty() && name.len() <= max && !name.contains('\0')
}

/// Can `name` be passed to Steam as a stat / achievement API name? Non-empty, at most
/// [`MAX_API_NAME_BYTES`] bytes, and no NUL byte.
#[cfg_attr(not(feature = "stats"), allow(dead_code))]
pub fn is_valid_api_name(name: &str) -> bool {
    valid(name, MAX_API_NAME_BYTES)
}

/// Can `name` be passed to Steam as a leaderboard name? Non-empty, at most
/// [`MAX_LEADERBOARD_NAME_BYTES`] bytes, and no NUL byte.
#[cfg_attr(not(feature = "leaderboards"), allow(dead_code))]
pub fn is_valid_leaderboard_name(name: &str) -> bool {
    valid(name, MAX_LEADERBOARD_NAME_BYTES)
}
