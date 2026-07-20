use crate::token::TokenId;

pub type PairKey = u64;

pub const fn pack_pair(a: TokenId, b: TokenId) -> PairKey {
    ((a as u64) << 32) | (b as u64)
}

pub const fn unpack_pair(key: PairKey) -> (TokenId, TokenId) {
    ((key >> 32) as u32, key as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const BOUNDARIES: [TokenId; 8] = [
        0,
        65_534,
        65_535,
        65_536,
        65_537,
        131_070,
        131_071,
        u32::MAX,
    ];

    #[test]
    fn pair_round_trips_cover_u32_boundaries() {
        for a in BOUNDARIES {
            for b in BOUNDARIES {
                assert_eq!(unpack_pair(pack_pair(a, b)), (a, b));
            }
        }
    }

    #[test]
    fn generated_boundary_pairs_do_not_collide() {
        let mut keys = HashSet::new();
        for a in BOUNDARIES {
            for b in BOUNDARIES {
                assert!(keys.insert(pack_pair(a, b)), "collision for ({a}, {b})");
            }
        }
        assert_eq!(keys.len(), BOUNDARIES.len() * BOUNDARIES.len());
    }

    #[test]
    fn ordered_pairs_are_distinct_and_numerically_lexicographic() {
        assert_ne!(pack_pair(0, 1), pack_pair(1, 0));
        assert_ne!(pack_pair(65_535, 65_536), pack_pair(65_536, 65_535));
        assert!(pack_pair(65_535, u32::MAX) < pack_pair(65_536, 0));
    }
}
