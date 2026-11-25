pub mod catalog;
pub mod community;
pub mod opgg;

pub const STATS_EXCLUDED_GAME_MODES: &[&str] = &["CHERRY", "TFT", "STRAWBERRY"];
pub const ARENA_QUEUE_IDS: &[u32] = &[1700, 1710, 1750];
pub const CLASSIC_QUEUE_IDS: &[u32] = &[4310, 4320, 3260, 3262];
pub const ARAM_MAYHEM_CLASSIC_QUEUE_IDS: &[u32] = &[2450, 3280];

pub fn is_stats_enabled(game_mode: &str) -> bool {
    !STATS_EXCLUDED_GAME_MODES
        .iter()
        .any(|excluded| excluded.eq_ignore_ascii_case(game_mode))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_non_summoners_rift_modes() {
        assert!(!is_stats_enabled("CHERRY"));
        assert!(!is_stats_enabled("TFT"));
        assert!(is_stats_enabled("CLASSIC"));
        assert!(is_stats_enabled("ARAM"));
    }
}
