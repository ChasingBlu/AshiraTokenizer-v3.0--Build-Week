use std::error::Error;
use std::fmt;

pub type TokenId = u32;

pub const SPECIAL_TOKEN_COUNT: u32 = 20;
pub const SPECIAL_TOKEN_START: TokenId = 0;
pub const SPECIAL_TOKEN_END: TokenId = 19;
pub const BYTE_TOKEN_START: TokenId = 20;
pub const BYTE_TOKEN_END: TokenId = 275;
pub const BPE_TOKEN_START: TokenId = 276;
pub const BPE_TOKEN_START_INDEX: usize = 276;
pub const MAX_VOCAB_SIZE: u32 = 131_072;
pub const MAX_VOCAB_SIZE_USIZE: usize = 131_072;
pub const MAX_TOKEN_ID: TokenId = 131_071;

pub const TOKEN_PAD: TokenId = 0;
pub const TOKEN_UNK: TokenId = 1;
pub const TOKEN_BOS: TokenId = 2;
pub const TOKEN_EOS: TokenId = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanonicalSpecialToken {
    pub id: TokenId,
    pub bytes: &'static [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpecialTokenAlias {
    pub bytes: &'static [u8],
    pub id: TokenId,
}

pub const CANONICAL_SPECIAL_TOKENS: [CanonicalSpecialToken; 20] = [
    CanonicalSpecialToken {
        id: 0,
        bytes: b"<PAD>",
    },
    CanonicalSpecialToken {
        id: 1,
        bytes: b"<UNK>",
    },
    CanonicalSpecialToken {
        id: 2,
        bytes: b"<BOS>",
    },
    CanonicalSpecialToken {
        id: 3,
        bytes: b"<EOS>",
    },
    CanonicalSpecialToken {
        id: 4,
        bytes: b"<kareem_narration>",
    },
    CanonicalSpecialToken {
        id: 5,
        bytes: b"<dylan_thinking>",
    },
    CanonicalSpecialToken {
        id: 6,
        bytes: b"<DYLAN>",
    },
    CanonicalSpecialToken {
        id: 7,
        bytes: b"<DYLAN_ADVERSARIAL>",
    },
    CanonicalSpecialToken {
        id: 8,
        bytes: b"<BLU>",
    },
    CanonicalSpecialToken {
        id: 9,
        bytes: b"<ECHO>",
    },
    CanonicalSpecialToken {
        id: 10,
        bytes: b"<RESONANCE>",
    },
    CanonicalSpecialToken {
        id: 11,
        bytes: b"<AI>",
    },
    CanonicalSpecialToken {
        id: 12,
        bytes: b"<PHIL>",
    },
    CanonicalSpecialToken {
        id: 13,
        bytes: b"<SYM>",
    },
    CanonicalSpecialToken {
        id: 14,
        bytes: b"<REFLECTION>",
    },
    CanonicalSpecialToken {
        id: 15,
        bytes: b"<CAIROS>",
    },
    CanonicalSpecialToken {
        id: 16,
        bytes: b"[[/ANCHOR]]",
    },
    CanonicalSpecialToken {
        id: 17,
        bytes: b"[[/CSA]]",
    },
    CanonicalSpecialToken {
        id: 18,
        bytes: b"<science_doc>",
    },
    CanonicalSpecialToken { id: 19, bytes: b"" },
];

// Byte-exact public-v2 code/fixture aliases. Native artifacts store only the
// canonical bytes above; aliases never become independent vocabulary records.
pub const SPECIAL_TOKEN_ALIASES: [SpecialTokenAlias; 44] = [
    SpecialTokenAlias {
        bytes: b"<PAD>",
        id: TOKEN_PAD,
    },
    SpecialTokenAlias {
        bytes: b"<UNK>",
        id: TOKEN_UNK,
    },
    SpecialTokenAlias {
        bytes: b"<BOS>",
        id: TOKEN_BOS,
    },
    SpecialTokenAlias {
        bytes: b"<EOS>",
        id: TOKEN_EOS,
    },
    SpecialTokenAlias {
        bytes: b"<KAREEM>",
        id: 4,
    },
    SpecialTokenAlias {
        bytes: b"<DYLAN_THINKING>",
        id: 5,
    },
    SpecialTokenAlias {
        bytes: b"<DYLAN_RESPONSE>",
        id: 6,
    },
    SpecialTokenAlias {
        bytes: b"<DYLAN_ADVERSARIAL>",
        id: 7,
    },
    SpecialTokenAlias {
        bytes: b"<BLU>",
        id: 8,
    },
    SpecialTokenAlias {
        bytes: b"<ECHO>",
        id: 9,
    },
    SpecialTokenAlias {
        bytes: b"<RESONANCE>",
        id: 10,
    },
    SpecialTokenAlias {
        bytes: b"<AI>",
        id: 11,
    },
    SpecialTokenAlias {
        bytes: b"<PHIL>",
        id: 12,
    },
    SpecialTokenAlias {
        bytes: b"<SYM>",
        id: 13,
    },
    SpecialTokenAlias {
        bytes: b"<REFLECTION>",
        id: 14,
    },
    SpecialTokenAlias {
        bytes: b"<CAIROS>",
        id: 15,
    },
    SpecialTokenAlias {
        bytes: b"[[ANCHOR]]",
        id: 16,
    },
    SpecialTokenAlias {
        bytes: b"[[CSA]]",
        id: 17,
    },
    SpecialTokenAlias {
        bytes: b"<science_doc>",
        id: 18,
    },
    SpecialTokenAlias {
        bytes: b"</KAREEM>",
        id: 4,
    },
    SpecialTokenAlias {
        bytes: b"</DYLAN_THINKING>",
        id: 5,
    },
    SpecialTokenAlias {
        bytes: b"</DYLAN_RESPONSE>",
        id: 6,
    },
    SpecialTokenAlias {
        bytes: b"</DYLAN_ADVERSARIAL>",
        id: 7,
    },
    SpecialTokenAlias {
        bytes: b"</BLU>",
        id: 8,
    },
    SpecialTokenAlias {
        bytes: b"</ECHO>",
        id: 9,
    },
    SpecialTokenAlias {
        bytes: b"</RESONANCE>",
        id: 10,
    },
    SpecialTokenAlias {
        bytes: b"</AI>",
        id: 11,
    },
    SpecialTokenAlias {
        bytes: b"</PHIL>",
        id: 12,
    },
    SpecialTokenAlias {
        bytes: b"</SYM>",
        id: 13,
    },
    SpecialTokenAlias {
        bytes: b"</REFLECTION>",
        id: 14,
    },
    SpecialTokenAlias {
        bytes: b"</CAIROS>",
        id: 15,
    },
    SpecialTokenAlias {
        bytes: b"[[/ANCHOR]]",
        id: 16,
    },
    SpecialTokenAlias {
        bytes: b"[[/CSA]]",
        id: 17,
    },
    SpecialTokenAlias {
        bytes: b"</science_doc>",
        id: 18,
    },
    SpecialTokenAlias {
        bytes: b"<kareem_response>",
        id: 4,
    },
    SpecialTokenAlias {
        bytes: b"</kareem_response>",
        id: 4,
    },
    SpecialTokenAlias {
        bytes: b"<kareem_narration>",
        id: 4,
    },
    SpecialTokenAlias {
        bytes: b"</kareem_narration>",
        id: 4,
    },
    SpecialTokenAlias {
        bytes: b"<dylan_thinking>",
        id: 5,
    },
    SpecialTokenAlias {
        bytes: b"</dylan_thinking>",
        id: 5,
    },
    SpecialTokenAlias {
        bytes: b"<dylan_response>",
        id: 6,
    },
    SpecialTokenAlias {
        bytes: b"</dylan_response>",
        id: 6,
    },
    SpecialTokenAlias {
        bytes: b"<DYLAN>",
        id: 6,
    },
    SpecialTokenAlias {
        bytes: b"</DYLAN>",
        id: 6,
    },
];

// Compatibility name retained for the inherited CLI while the validated v3
// configuration surface is introduced. It now denotes the locked 128K target.
pub const VOCAB_SIZE: usize = MAX_VOCAB_SIZE_USIZE;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TokenError {
    VocabTargetBelowBase { target: usize, minimum: u32 },
    VocabTargetTooLarge { target: usize, maximum: u32 },
    InvalidTokenId { id: TokenId, maximum: TokenId },
    VocabularyExhausted { vocab_len: usize, maximum: u32 },
    TokenIndexNotRepresentable { id: TokenId },
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VocabTargetBelowBase { target, minimum } => write!(
                f,
                "vocabulary target {target} is below the mandatory base size {minimum}"
            ),
            Self::VocabTargetTooLarge { target, maximum } => write!(
                f,
                "vocabulary target {target} exceeds the operational maximum {maximum}"
            ),
            Self::InvalidTokenId { id, maximum } => {
                write!(f, "token ID {id} exceeds the operational maximum {maximum}")
            }
            Self::VocabularyExhausted { vocab_len, maximum } => write!(
                f,
                "cannot allocate token ID from vocabulary length {vocab_len}; maximum entry count is {maximum}"
            ),
            Self::TokenIndexNotRepresentable { id } => {
                write!(f, "token ID {id} is not representable as a platform index")
            }
        }
    }
}

impl Error for TokenError {}

pub fn validate_vocab_target(target: usize) -> Result<u32, TokenError> {
    let target_u32 = u32::try_from(target).map_err(|_| TokenError::VocabTargetTooLarge {
        target,
        maximum: MAX_VOCAB_SIZE,
    })?;
    if target_u32 < BPE_TOKEN_START {
        return Err(TokenError::VocabTargetBelowBase {
            target,
            minimum: BPE_TOKEN_START,
        });
    }
    if target_u32 > MAX_VOCAB_SIZE {
        return Err(TokenError::VocabTargetTooLarge {
            target,
            maximum: MAX_VOCAB_SIZE,
        });
    }
    Ok(target_u32)
}

pub fn validate_token_id(id: TokenId) -> Result<(), TokenError> {
    if id > MAX_TOKEN_ID {
        return Err(TokenError::InvalidTokenId {
            id,
            maximum: MAX_TOKEN_ID,
        });
    }
    Ok(())
}

pub fn allocate_token_id(vocab_len: usize) -> Result<TokenId, TokenError> {
    if vocab_len >= MAX_VOCAB_SIZE_USIZE {
        return Err(TokenError::VocabularyExhausted {
            vocab_len,
            maximum: MAX_VOCAB_SIZE,
        });
    }
    let id = u32::try_from(vocab_len).map_err(|_| TokenError::VocabularyExhausted {
        vocab_len,
        maximum: MAX_VOCAB_SIZE,
    })?;
    validate_token_id(id)?;
    Ok(id)
}

pub fn token_id_to_index(id: TokenId) -> Result<usize, TokenError> {
    validate_token_id(id)?;
    usize::try_from(id).map_err(|_| TokenError::TokenIndexNotRepresentable { id })
}

pub fn base_byte_token(byte: u8) -> TokenId {
    BYTE_TOKEN_START + u32::from(byte)
}

pub fn canonical_special_bytes(id: TokenId) -> Option<&'static [u8]> {
    let index = usize::try_from(id).ok()?;
    let special = CANONICAL_SPECIAL_TOKENS.get(index)?;
    (special.id == id).then_some(special.bytes)
}

pub fn special_token_id(bytes: &[u8]) -> Option<TokenId> {
    SPECIAL_TOKEN_ALIASES
        .iter()
        .find(|alias| alias.bytes == bytes)
        .map(|alias| alias.id)
}

pub fn match_special_alias_prefix(input: &[u8]) -> Option<(TokenId, usize)> {
    let mut best: Option<&SpecialTokenAlias> = None;
    for alias in &SPECIAL_TOKEN_ALIASES {
        if !input.starts_with(alias.bytes) {
            continue;
        }
        let replace = match best {
            None => true,
            Some(current) => {
                alias.bytes.len() > current.bytes.len()
                    || (alias.bytes.len() == current.bytes.len() && alias.bytes < current.bytes)
            }
        };
        if replace {
            best = Some(alias);
        }
    }
    best.map(|alias| (alias.id, alias.bytes.len()))
}

pub fn is_special_alias_sequence(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let mut position = 0usize;
    while position < bytes.len() {
        let Some((_, matched_len)) = match_special_alias_prefix(&bytes[position..]) else {
            return false;
        };
        position = match position.checked_add(matched_len) {
            Some(next) => next,
            None => return false,
        };
    }
    position == bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn operational_id_boundaries_are_checked() {
        for id in [65_535, 65_536, 131_071] {
            assert_eq!(validate_token_id(id), Ok(()));
        }
        assert!(matches!(
            validate_token_id(131_072),
            Err(TokenError::InvalidTokenId { id: 131_072, .. })
        ));
        assert!(matches!(
            validate_token_id(u32::MAX),
            Err(TokenError::InvalidTokenId { id: u32::MAX, .. })
        ));
    }

    #[test]
    fn vocabulary_target_boundaries_are_checked() {
        assert_eq!(validate_vocab_target(131_072), Ok(131_072));
        assert!(matches!(
            validate_vocab_target(131_073),
            Err(TokenError::VocabTargetTooLarge {
                target: 131_073,
                ..
            })
        ));
        assert_eq!(validate_vocab_target(276), Ok(276));
        assert!(matches!(
            validate_vocab_target(275),
            Err(TokenError::VocabTargetBelowBase { target: 275, .. })
        ));
    }

    #[test]
    fn checked_allocation_crosses_the_old_u16_ceiling() {
        assert_eq!(allocate_token_id(65_535), Ok(65_535));
        assert_eq!(allocate_token_id(65_536), Ok(65_536));
        assert_eq!(allocate_token_id(131_071), Ok(131_071));
        assert!(matches!(
            allocate_token_id(131_072),
            Err(TokenError::VocabularyExhausted {
                vocab_len: 131_072,
                ..
            })
        ));
        assert!(allocate_token_id(usize::MAX).is_err());
    }

    #[test]
    fn byte_tokens_fill_the_locked_base_range() {
        for byte in u8::MIN..=u8::MAX {
            assert_eq!(base_byte_token(byte), BYTE_TOKEN_START + u32::from(byte));
        }
        assert_eq!(base_byte_token(u8::MIN), BYTE_TOKEN_START);
        assert_eq!(base_byte_token(u8::MAX), BYTE_TOKEN_END);
    }

    #[test]
    fn canonical_special_bytes_match_the_locked_table() {
        let expected: [&[u8]; 20] = [
            b"<PAD>",
            b"<UNK>",
            b"<BOS>",
            b"<EOS>",
            b"<kareem_narration>",
            b"<dylan_thinking>",
            b"<DYLAN>",
            b"<DYLAN_ADVERSARIAL>",
            b"<BLU>",
            b"<ECHO>",
            b"<RESONANCE>",
            b"<AI>",
            b"<PHIL>",
            b"<SYM>",
            b"<REFLECTION>",
            b"<CAIROS>",
            b"[[/ANCHOR]]",
            b"[[/CSA]]",
            b"<science_doc>",
            b"",
        ];
        for (index, expected_bytes) in expected.iter().enumerate() {
            let id = u32::try_from(index).expect("special index fits TokenId");
            assert_eq!(canonical_special_bytes(id), Some(*expected_bytes));
            assert_eq!(CANONICAL_SPECIAL_TOKENS[index].id, id);
        }
        assert_eq!(canonical_special_bytes(20), None);
        assert_eq!(canonical_special_bytes(u32::MAX), None);
    }

    #[test]
    fn locked_aliases_are_byte_distinct_and_decode_canonically() {
        let mut seen = HashSet::new();
        for alias in SPECIAL_TOKEN_ALIASES {
            assert!(!alias.bytes.is_empty());
            assert!(
                seen.insert(alias.bytes),
                "duplicate alias: {:?}",
                alias.bytes
            );
            assert_eq!(special_token_id(alias.bytes), Some(alias.id));
            assert!(canonical_special_bytes(alias.id).is_some());
        }
        assert_eq!(seen.len(), 44);
        assert_eq!(special_token_id(b""), None);
        assert_eq!(special_token_id(b"<not-a-special>"), None);

        assert_eq!(special_token_id(b"<KAREEM>"), Some(4));
        assert_eq!(
            canonical_special_bytes(4),
            Some(b"<kareem_narration>".as_slice())
        );
        assert_ne!(b"<KAREEM>".as_slice(), canonical_special_bytes(4).unwrap());
        assert_eq!(special_token_id(b"[[ANCHOR]]"), Some(16));
        assert_eq!(canonical_special_bytes(16), Some(b"[[/ANCHOR]]".as_slice()));
    }

    #[test]
    fn reserved_id_19_is_empty_and_has_no_alias() {
        assert_eq!(canonical_special_bytes(19), Some(b"".as_slice()));
        assert!(!SPECIAL_TOKEN_ALIASES.iter().any(|alias| alias.id == 19));
    }

    #[test]
    fn special_only_matching_is_byte_exact_and_concatenation_aware() {
        assert!(is_special_alias_sequence(b"<KAREEM></KAREEM>"));
        assert!(is_special_alias_sequence(b"[[ANCHOR]][[/ANCHOR]]"));
        assert!(is_special_alias_sequence(
            b"<kareem_narration><dylan_response>"
        ));
        assert!(!is_special_alias_sequence(b""));
        assert!(!is_special_alias_sequence(b"<KAREEM> "));
        assert!(!is_special_alias_sequence(b"<KAREEM>text"));
        assert!(!is_special_alias_sequence(b"<kAreem>"));
    }
}
