use crate::artifact::Tokenizer;
use crate::presegment::{LosslessPresegment, PresegmentError, visit_lossless_presegments};
use crate::token::{
    BPE_TOKEN_START, MAX_TOKEN_ID, TokenId, base_byte_token, match_special_alias_prefix,
    validate_token_id,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::error::Error;
use std::fmt;
use std::io::{self, Write};

pub const ENCODED_TOKENS_SCHEMA: &str = "ashira_v3_encoded_tokens_v1";
pub const ENCODED_TOKEN_ID_WIDTH: &str = "u32";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodecLimits {
    pub max_input_bytes: u64,
    pub max_token_count: u64,
    pub max_decoded_bytes: u64,
    pub max_encoded_json_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodecError {
    ResourceLimitExceeded {
        resource: &'static str,
        limit: u64,
        actual: u64,
    },
    ArithmeticOverflow {
        operation: &'static str,
    },
    AllocationFailed {
        resource: &'static str,
    },
    InvalidTokenId {
        id: TokenId,
        vocab_size: u32,
    },
    Presegment {
        message: String,
    },
    Json {
        message: String,
    },
    SchemaMismatch,
    TokenIdWidthMismatch,
    TokenCountMismatch,
    TokenizerBindingMismatch,
    DecodedLengthMismatch,
    DecodedDigestMismatch,
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResourceLimitExceeded {
                resource,
                limit,
                actual,
            } => write!(
                formatter,
                "codec resource limit exceeded for {resource}: limit {limit}, actual {actual}"
            ),
            Self::ArithmeticOverflow { operation } => {
                write!(formatter, "codec arithmetic overflow: {operation}")
            }
            Self::AllocationFailed { resource } => {
                write!(formatter, "codec allocation failed for {resource}")
            }
            Self::InvalidTokenId { id, vocab_size } => write!(
                formatter,
                "token ID {id} is invalid for vocabulary size {vocab_size}"
            ),
            Self::Presegment { message } => write!(formatter, "pre-segment failure: {message}"),
            Self::Json { message } => write!(formatter, "encoded-token JSON failure: {message}"),
            Self::SchemaMismatch => write!(formatter, "encoded-token JSON schema mismatch"),
            Self::TokenIdWidthMismatch => {
                write!(formatter, "encoded-token JSON token ID width mismatch")
            }
            Self::TokenCountMismatch => write!(formatter, "encoded-token count mismatch"),
            Self::TokenizerBindingMismatch => write!(formatter, "tokenizer binding mismatch"),
            Self::DecodedLengthMismatch => write!(formatter, "decoded byte length mismatch"),
            Self::DecodedDigestMismatch => write!(formatter, "decoded byte digest mismatch"),
        }
    }
}

impl Error for CodecError {}

impl From<PresegmentError> for CodecError {
    fn from(error: PresegmentError) -> Self {
        Self::Presegment {
            message: error.to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedTokensV1 {
    tokenizer_vocab_size: u32,
    tokenizer_merge_count: u64,
    tokenizer_sequence_sha256: [u8; 32],
    decoded_bytes: u64,
    decoded_sha256: [u8; 32],
    token_ids: Vec<TokenId>,
}

impl EncodedTokensV1 {
    pub fn encode(
        tokenizer: &Tokenizer,
        input: &[u8],
        limits: &CodecLimits,
    ) -> Result<Self, CodecError> {
        let token_ids = tokenizer.encode(input, limits)?;
        let (decoded_bytes, decoded_sha256) = tokenizer.decoded_evidence(&token_ids, limits)?;
        Ok(Self {
            tokenizer_vocab_size: tokenizer_vocab_size(tokenizer)?,
            tokenizer_merge_count: tokenizer_merge_count(tokenizer)?,
            tokenizer_sequence_sha256: tokenizer_sequence_sha256(tokenizer)?,
            decoded_bytes,
            decoded_sha256,
            token_ids,
        })
    }

    pub fn parse_json(bytes: &[u8], limits: &CodecLimits) -> Result<Self, CodecError> {
        enforce_len_limit(
            bytes.len(),
            limits.max_encoded_json_bytes,
            "encoded_json_bytes",
        )?;
        let wire: EncodedTokensWire =
            serde_json::from_slice(bytes).map_err(|error| CodecError::Json {
                message: error.to_string(),
            })?;
        if wire.schema != ENCODED_TOKENS_SCHEMA {
            return Err(CodecError::SchemaMismatch);
        }
        if wire.token_id_width != ENCODED_TOKEN_ID_WIDTH {
            return Err(CodecError::TokenIdWidthMismatch);
        }
        let token_count = checked_len(wire.token_ids.len(), "parsed token count")?;
        if wire.token_count != token_count {
            return Err(CodecError::TokenCountMismatch);
        }
        enforce_u64_limit(token_count, limits.max_token_count, "encoded_token_count")?;
        enforce_u64_limit(
            wire.decoded_bytes,
            limits.max_decoded_bytes,
            "decoded_bytes",
        )?;
        let vocab_size = wire.tokenizer.vocab_size;
        validate_binding_counts(vocab_size, wire.tokenizer.merge_count)?;
        for &id in &wire.token_ids {
            if validate_token_id(id).is_err() || id >= vocab_size {
                return Err(CodecError::InvalidTokenId { id, vocab_size });
            }
        }

        Ok(Self {
            tokenizer_vocab_size: vocab_size,
            tokenizer_merge_count: wire.tokenizer.merge_count,
            tokenizer_sequence_sha256: decode_upper_hex_32(
                &wire.tokenizer.sequence_sha256,
                "tokenizer.sequence_sha256",
            )?,
            decoded_bytes: wire.decoded_bytes,
            decoded_sha256: decode_upper_hex_32(&wire.decoded_sha256, "decoded_sha256")?,
            token_ids: wire.token_ids,
        })
    }

    pub fn to_canonical_json(&self, limits: &CodecLimits) -> Result<Vec<u8>, CodecError> {
        self.validate_internal(limits)?;
        let wire = EncodedTokensWireRef {
            schema: ENCODED_TOKENS_SCHEMA,
            token_id_width: ENCODED_TOKEN_ID_WIDTH,
            tokenizer: TokenizerBindingWireRef {
                vocab_size: self.tokenizer_vocab_size,
                merge_count: self.tokenizer_merge_count,
                sequence_sha256: encode_upper_hex(&self.tokenizer_sequence_sha256),
            },
            decoded_bytes: self.decoded_bytes,
            decoded_sha256: encode_upper_hex(&self.decoded_sha256),
            token_count: checked_len(self.token_ids.len(), "serialized token count")?,
            token_ids: &self.token_ids,
        };
        let mut writer = BoundedJsonWriter::new(limits.max_encoded_json_bytes);
        if let Err(error) = serde_json::to_writer(&mut writer, &wire) {
            return Err(writer.into_codec_error(error));
        }
        if let Err(error) = writer.write_all(b"\n") {
            return Err(writer.into_codec_error(serde_json::Error::io(error)));
        }
        Ok(writer.into_bytes())
    }

    pub fn decode(
        &self,
        tokenizer: &Tokenizer,
        limits: &CodecLimits,
    ) -> Result<Vec<u8>, CodecError> {
        self.validate_internal(limits)?;
        if self.tokenizer_vocab_size != tokenizer_vocab_size(tokenizer)?
            || self.tokenizer_merge_count != tokenizer_merge_count(tokenizer)?
            || self.tokenizer_sequence_sha256 != tokenizer_sequence_sha256(tokenizer)?
        {
            return Err(CodecError::TokenizerBindingMismatch);
        }
        let decoded = tokenizer.decode(&self.token_ids, limits)?;
        if checked_len(decoded.len(), "decoded output byte length")? != self.decoded_bytes {
            return Err(CodecError::DecodedLengthMismatch);
        }
        if sha256(&decoded) != self.decoded_sha256 {
            return Err(CodecError::DecodedDigestMismatch);
        }
        Ok(decoded)
    }

    pub fn token_ids(&self) -> &[TokenId] {
        &self.token_ids
    }

    pub const fn decoded_bytes(&self) -> u64 {
        self.decoded_bytes
    }

    pub const fn decoded_sha256(&self) -> [u8; 32] {
        self.decoded_sha256
    }

    fn validate_internal(&self, limits: &CodecLimits) -> Result<(), CodecError> {
        let token_count = checked_len(self.token_ids.len(), "encoded token count")?;
        enforce_u64_limit(token_count, limits.max_token_count, "encoded_token_count")?;
        enforce_u64_limit(
            self.decoded_bytes,
            limits.max_decoded_bytes,
            "decoded_bytes",
        )?;
        validate_binding_counts(self.tokenizer_vocab_size, self.tokenizer_merge_count)?;
        for &id in &self.token_ids {
            if validate_token_id(id).is_err() || id >= self.tokenizer_vocab_size {
                return Err(CodecError::InvalidTokenId {
                    id,
                    vocab_size: self.tokenizer_vocab_size,
                });
            }
        }
        Ok(())
    }
}

impl Tokenizer {
    pub fn encode(&self, input: &[u8], limits: &CodecLimits) -> Result<Vec<TokenId>, CodecError> {
        enforce_len_limit(input.len(), limits.max_input_bytes, "input_bytes")?;
        let mut encoded = Vec::new();
        let reserve = usize::try_from(limits.max_token_count)
            .unwrap_or(usize::MAX)
            .min(input.len());
        encoded
            .try_reserve(reserve)
            .map_err(|_| CodecError::AllocationFailed {
                resource: "encoded_tokens",
            })?;

        let mut consumer_error = None;
        let traversal = visit_lossless_presegments(input, |piece| {
            let result = match piece {
                LosslessPresegment::Mergeable(segment) => {
                    self.encode_mergeable_segment(segment, &mut encoded, limits)
                }
                LosslessPresegment::LiteralByte(byte) => {
                    push_token(&mut encoded, base_byte_token(byte), limits)
                }
            };
            if let Err(error) = result {
                let message = error.to_string();
                consumer_error = Some(error);
                return Err(PresegmentError::Consumer { message });
            }
            Ok(())
        });
        if let Some(error) = consumer_error {
            return Err(error);
        }
        traversal.map_err(CodecError::from)?;
        Ok(encoded)
    }

    pub fn decode(
        &self,
        token_ids: &[TokenId],
        limits: &CodecLimits,
    ) -> Result<Vec<u8>, CodecError> {
        let vocab_size = tokenizer_vocab_size(self)?;
        let (decoded_bytes, _) = self.decoded_evidence(token_ids, limits)?;
        let capacity =
            usize::try_from(decoded_bytes).map_err(|_| CodecError::ResourceLimitExceeded {
                resource: "decoded_bytes_platform_index",
                limit: usize::MAX as u64,
                actual: decoded_bytes,
            })?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(capacity)
            .map_err(|_| CodecError::AllocationFailed {
                resource: "decoded_bytes",
            })?;
        for &id in token_ids {
            let bytes = self
                .token_bytes(id)
                .ok_or(CodecError::InvalidTokenId { id, vocab_size })?;
            output.extend_from_slice(bytes);
        }
        Ok(output)
    }

    fn decoded_evidence(
        &self,
        token_ids: &[TokenId],
        limits: &CodecLimits,
    ) -> Result<(u64, [u8; 32]), CodecError> {
        enforce_len_limit(
            token_ids.len(),
            limits.max_token_count,
            "encoded_token_count",
        )?;
        let vocab_size = tokenizer_vocab_size(self)?;
        let mut decoded_bytes = 0u64;
        let mut hasher = Sha256::new();
        for &id in token_ids {
            validate_codec_id(id, vocab_size)?;
            let token_bytes = self
                .token_bytes(id)
                .ok_or(CodecError::InvalidTokenId { id, vocab_size })?;
            decoded_bytes = decoded_bytes
                .checked_add(checked_len(token_bytes.len(), "token byte length")?)
                .ok_or(CodecError::ArithmeticOverflow {
                    operation: "decoded byte count",
                })?;
            enforce_u64_limit(decoded_bytes, limits.max_decoded_bytes, "decoded_bytes")?;
            hasher.update(token_bytes);
        }
        Ok((decoded_bytes, finalize_sha256(hasher)))
    }

    fn encode_mergeable_segment(
        &self,
        segment: &[u8],
        output: &mut Vec<TokenId>,
        limits: &CodecLimits,
    ) -> Result<(), CodecError> {
        let mut cursor = 0usize;
        let mut ordinary_start = 0usize;
        while cursor < segment.len() {
            if let Some((special_id, matched_len)) = match_special_alias_prefix(&segment[cursor..])
            {
                self.encode_ordinary_bytes(&segment[ordinary_start..cursor], output, limits)?;
                push_token(output, special_id, limits)?;
                cursor = cursor
                    .checked_add(matched_len)
                    .ok_or(CodecError::ArithmeticOverflow {
                        operation: "special alias cursor",
                    })?;
                ordinary_start = cursor;
            } else {
                cursor = cursor
                    .checked_add(1)
                    .ok_or(CodecError::ArithmeticOverflow {
                        operation: "ordinary byte cursor",
                    })?;
            }
        }
        self.encode_ordinary_bytes(&segment[ordinary_start..], output, limits)
    }

    fn encode_ordinary_bytes(
        &self,
        bytes: &[u8],
        output: &mut Vec<TokenId>,
        limits: &CodecLimits,
    ) -> Result<(), CodecError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let mut nodes = Vec::new();
        nodes
            .try_reserve_exact(bytes.len())
            .map_err(|_| CodecError::AllocationFailed {
                resource: "merge_nodes",
            })?;
        for (index, byte) in bytes.iter().copied().enumerate() {
            nodes.push(MergeNode {
                token: base_byte_token(byte),
                previous: index.checked_sub(1),
                next: (index + 1 < bytes.len()).then_some(index + 1),
                live: true,
            });
        }

        let mut candidates = BinaryHeap::new();
        candidates
            .try_reserve(bytes.len().saturating_sub(1))
            .map_err(|_| CodecError::AllocationFailed {
                resource: "merge_candidates",
            })?;
        for left in 0..bytes.len().saturating_sub(1) {
            push_candidate(self, &nodes, left, left + 1, &mut candidates);
        }

        while let Some(candidate) = candidates.pop() {
            if !candidate.is_current(self, &nodes) {
                continue;
            }
            let previous = nodes[candidate.left].previous;
            let next = nodes[candidate.right].next;
            nodes[candidate.left].token = candidate.merged;
            nodes[candidate.left].next = next;
            nodes[candidate.right].live = false;
            if let Some(next_index) = next {
                nodes[next_index].previous = Some(candidate.left);
            }
            if let Some(previous_index) = previous {
                push_candidate(
                    self,
                    &nodes,
                    previous_index,
                    candidate.left,
                    &mut candidates,
                );
            }
            if let Some(next_index) = next {
                push_candidate(self, &nodes, candidate.left, next_index, &mut candidates);
            }
        }

        let mut current = Some(0usize);
        while let Some(index) = current {
            if !nodes[index].live {
                return Err(CodecError::ArithmeticOverflow {
                    operation: "merge linked-list traversal",
                });
            }
            push_token(output, nodes[index].token, limits)?;
            current = nodes[index].next;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct MergeNode {
    token: TokenId,
    previous: Option<usize>,
    next: Option<usize>,
    live: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MergeCandidate {
    merged: TokenId,
    left: usize,
    right: usize,
}

impl MergeCandidate {
    fn is_current(self, tokenizer: &Tokenizer, nodes: &[MergeNode]) -> bool {
        nodes.get(self.left).is_some_and(|node| node.live)
            && nodes.get(self.right).is_some_and(|node| node.live)
            && nodes[self.left].next == Some(self.right)
            && nodes[self.right].previous == Some(self.left)
            && tokenizer.merged_token(nodes[self.left].token, nodes[self.right].token)
                == Some(self.merged)
    }
}

impl Ord for MergeCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .merged
            .cmp(&self.merged)
            .then_with(|| other.left.cmp(&self.left))
            .then_with(|| other.right.cmp(&self.right))
    }
}

impl PartialOrd for MergeCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EncodedTokensWire {
    schema: String,
    token_id_width: String,
    tokenizer: TokenizerBindingWire,
    decoded_bytes: u64,
    decoded_sha256: String,
    token_count: u64,
    token_ids: Vec<TokenId>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TokenizerBindingWire {
    vocab_size: u32,
    merge_count: u64,
    sequence_sha256: String,
}

#[derive(Serialize)]
struct EncodedTokensWireRef<'a> {
    schema: &'static str,
    token_id_width: &'static str,
    tokenizer: TokenizerBindingWireRef,
    decoded_bytes: u64,
    decoded_sha256: String,
    token_count: u64,
    token_ids: &'a [TokenId],
}

#[derive(Serialize)]
struct TokenizerBindingWireRef {
    vocab_size: u32,
    merge_count: u64,
    sequence_sha256: String,
}

struct BoundedJsonWriter {
    bytes: Vec<u8>,
    limit: u64,
    failure: Option<JsonWriterFailure>,
}

#[derive(Clone, Copy)]
enum JsonWriterFailure {
    Limit { actual: u64 },
    Allocation,
    Arithmetic,
}

impl BoundedJsonWriter {
    fn new(limit: u64) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            failure: None,
        }
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    fn into_codec_error(self, error: serde_json::Error) -> CodecError {
        match self.failure {
            Some(JsonWriterFailure::Limit { actual }) => CodecError::ResourceLimitExceeded {
                resource: "encoded_json_bytes",
                limit: self.limit,
                actual,
            },
            Some(JsonWriterFailure::Allocation) => CodecError::AllocationFailed {
                resource: "encoded_json",
            },
            Some(JsonWriterFailure::Arithmetic) => CodecError::ArithmeticOverflow {
                operation: "encoded JSON byte count",
            },
            None => CodecError::Json {
                message: error.to_string(),
            },
        }
    }
}

impl Write for BoundedJsonWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let current = u64::try_from(self.bytes.len()).map_err(|_| {
            self.failure = Some(JsonWriterFailure::Arithmetic);
            io::Error::other("encoded JSON length is not representable")
        })?;
        let additional = u64::try_from(bytes.len()).map_err(|_| {
            self.failure = Some(JsonWriterFailure::Arithmetic);
            io::Error::other("encoded JSON write length is not representable")
        })?;
        let actual = current.checked_add(additional).ok_or_else(|| {
            self.failure = Some(JsonWriterFailure::Arithmetic);
            io::Error::other("encoded JSON length overflow")
        })?;
        if actual > self.limit {
            self.failure = Some(JsonWriterFailure::Limit { actual });
            return Err(io::Error::other("encoded JSON byte limit exceeded"));
        }
        self.bytes.try_reserve(bytes.len()).map_err(|_| {
            self.failure = Some(JsonWriterFailure::Allocation);
            io::Error::other("encoded JSON allocation failed")
        })?;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn push_candidate(
    tokenizer: &Tokenizer,
    nodes: &[MergeNode],
    left: usize,
    right: usize,
    candidates: &mut BinaryHeap<MergeCandidate>,
) {
    if let Some(merged) = tokenizer.merged_token(nodes[left].token, nodes[right].token) {
        candidates.push(MergeCandidate {
            merged,
            left,
            right,
        });
    }
}

fn push_token(
    output: &mut Vec<TokenId>,
    token: TokenId,
    limits: &CodecLimits,
) -> Result<(), CodecError> {
    let next_count = checked_len(output.len(), "encoded token count")?
        .checked_add(1)
        .ok_or(CodecError::ArithmeticOverflow {
            operation: "encoded token count",
        })?;
    enforce_u64_limit(next_count, limits.max_token_count, "encoded_token_count")?;
    output.push(token);
    Ok(())
}

fn validate_codec_id(id: TokenId, vocab_size: u32) -> Result<(), CodecError> {
    if validate_token_id(id).is_err() || id >= vocab_size {
        return Err(CodecError::InvalidTokenId { id, vocab_size });
    }
    Ok(())
}

fn validate_binding_counts(vocab_size: u32, merge_count: u64) -> Result<(), CodecError> {
    if !(BPE_TOKEN_START..=MAX_TOKEN_ID + 1).contains(&vocab_size)
        || merge_count != u64::from(vocab_size - BPE_TOKEN_START)
    {
        return Err(CodecError::TokenizerBindingMismatch);
    }
    Ok(())
}

fn tokenizer_vocab_size(tokenizer: &Tokenizer) -> Result<u32, CodecError> {
    u32::try_from(tokenizer.vocab_size()).map_err(|_| CodecError::ArithmeticOverflow {
        operation: "tokenizer vocabulary size conversion",
    })
}

fn tokenizer_merge_count(tokenizer: &Tokenizer) -> Result<u64, CodecError> {
    u64::try_from(tokenizer.merge_count()).map_err(|_| CodecError::ArithmeticOverflow {
        operation: "tokenizer merge count conversion",
    })
}

fn tokenizer_sequence_sha256(tokenizer: &Tokenizer) -> Result<[u8; 32], CodecError> {
    let mut hasher = Sha256::new();
    for ordinal in 0..tokenizer.merge_count() {
        let merge = tokenizer
            .merge_at(ordinal)
            .ok_or(CodecError::ArithmeticOverflow {
                operation: "immutable tokenizer merge sequence",
            })?;
        hasher.update(merge.a.to_le_bytes());
        hasher.update(merge.b.to_le_bytes());
        hasher.update(merge.merged.to_le_bytes());
    }
    Ok(finalize_sha256(hasher))
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    finalize_sha256(Sha256::new_with_prefix(bytes))
}

fn finalize_sha256(hasher: Sha256) -> [u8; 32] {
    let digest = hasher.finalize();
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&digest);
    bytes
}

fn encode_upper_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0F)]));
    }
    encoded
}

fn decode_upper_hex_32(value: &str, field: &'static str) -> Result<[u8; 32], CodecError> {
    let bytes = value.as_bytes();
    if bytes.len() != 64 {
        return Err(CodecError::Json {
            message: format!("{field} must contain exactly 64 uppercase hexadecimal digits"),
        });
    }
    let mut decoded = [0u8; 32];
    for (index, pair) in bytes.chunks_exact(2).enumerate() {
        let high = decode_upper_nibble(pair[0]).ok_or_else(|| CodecError::Json {
            message: format!("{field} must use uppercase hexadecimal"),
        })?;
        let low = decode_upper_nibble(pair[1]).ok_or_else(|| CodecError::Json {
            message: format!("{field} must use uppercase hexadecimal"),
        })?;
        decoded[index] = (high << 4) | low;
    }
    Ok(decoded)
}

const fn decode_upper_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn checked_len(length: usize, operation: &'static str) -> Result<u64, CodecError> {
    u64::try_from(length).map_err(|_| CodecError::ArithmeticOverflow { operation })
}

fn enforce_len_limit(length: usize, limit: u64, resource: &'static str) -> Result<(), CodecError> {
    enforce_u64_limit(
        checked_len(length, "resource length conversion")?,
        limit,
        resource,
    )
}

fn enforce_u64_limit(actual: u64, limit: u64, resource: &'static str) -> Result<(), CodecError> {
    if actual > limit {
        return Err(CodecError::ResourceLimitExceeded {
            resource,
            limit,
            actual,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BpeMerge;
    use crate::artifact::{ArtifactError, ArtifactHeaderV3};
    use crate::token::BPE_TOKEN_START;

    fn limits() -> CodecLimits {
        CodecLimits {
            max_input_bytes: 4096,
            max_token_count: 4096,
            max_decoded_bytes: 4096,
            max_encoded_json_bytes: 65_536,
        }
    }

    fn tokenizer(merges: &[(u8, u8)]) -> Tokenizer {
        let mut vocab = Vec::new();
        for id in 0..BPE_TOKEN_START {
            let bytes = if id < 20 {
                crate::token::canonical_special_bytes(id)
                    .expect("special bytes")
                    .to_vec()
            } else {
                vec![u8::try_from(id - 20).expect("base byte")]
            };
            vocab.push(bytes);
        }
        let mut records = Vec::new();
        for &(a, b) in merges {
            let left = base_byte_token(a);
            let right = base_byte_token(b);
            let merged = BPE_TOKEN_START + u32::try_from(records.len()).expect("merge ordinal");
            let mut bytes = vocab[usize::try_from(left).expect("left index")].clone();
            bytes.extend_from_slice(&vocab[usize::try_from(right).expect("right index")]);
            vocab.push(bytes);
            records.push(BpeMerge {
                a: left,
                b: right,
                merged,
            });
        }
        Tokenizer::try_from_parts(vocab, records).expect("test tokenizer")
    }

    fn ranked_tokenizer() -> Result<Tokenizer, ArtifactError> {
        let mut vocab = Vec::new();
        for id in 0..BPE_TOKEN_START {
            let bytes = if id < 20 {
                crate::token::canonical_special_bytes(id)
                    .expect("special bytes")
                    .to_vec()
            } else {
                vec![u8::try_from(id - 20).expect("base byte")]
            };
            vocab.push(bytes);
        }
        let ab = BPE_TOKEN_START;
        vocab.push(b"ab".to_vec());
        let bc = BPE_TOKEN_START + 1;
        vocab.push(b"bc".to_vec());
        let abc = BPE_TOKEN_START + 2;
        vocab.push(b"abc".to_vec());
        Tokenizer::try_from_parts(
            vocab,
            vec![
                BpeMerge {
                    a: base_byte_token(b'a'),
                    b: base_byte_token(b'b'),
                    merged: ab,
                },
                BpeMerge {
                    a: base_byte_token(b'b'),
                    b: base_byte_token(b'c'),
                    merged: bc,
                },
                BpeMerge {
                    a: ab,
                    b: base_byte_token(b'c'),
                    merged: abc,
                },
            ],
        )
    }

    fn cross_u16_tokenizer() -> Tokenizer {
        const TARGET_ID: TokenId = 65_536;
        let mut vocab = Vec::new();
        for id in 0..BPE_TOKEN_START {
            let bytes = if id < 20 {
                crate::token::canonical_special_bytes(id)
                    .expect("special bytes")
                    .to_vec()
            } else {
                vec![u8::try_from(id - 20).expect("base byte")]
            };
            vocab.push(bytes);
        }
        let merge_count =
            usize::try_from(TARGET_ID - BPE_TOKEN_START + 1).expect("cross-u16 merge count");
        let mut records = Vec::with_capacity(merge_count);
        for ordinal in 0..merge_count {
            let ordinal_u32 = u32::try_from(ordinal).expect("merge ordinal");
            let left_byte = u8::try_from(ordinal_u32 / 256).expect("left byte");
            let right_byte = u8::try_from(ordinal_u32 % 256).expect("right byte");
            vocab.push(vec![left_byte, right_byte]);
            records.push(BpeMerge {
                a: base_byte_token(left_byte),
                b: base_byte_token(right_byte),
                merged: BPE_TOKEN_START + ordinal_u32,
            });
        }
        Tokenizer::try_from_parts(vocab, records).expect("cross-u16 tokenizer")
    }

    #[test]
    fn deterministic_ranked_merges_are_leftmost_and_non_quadratic_structure() {
        let tokenizer = ranked_tokenizer().expect("ranked tokenizer");
        assert_eq!(
            tokenizer.encode(b"abcabc", &limits()).unwrap(),
            [BPE_TOKEN_START + 2, BPE_TOKEN_START + 2]
        );
        assert_eq!(
            tokenizer.encode(b"abab", &limits()).unwrap(),
            [BPE_TOKEN_START, BPE_TOKEN_START]
        );
    }

    #[test]
    fn line_boundaries_are_lossless_and_never_merge_across_structural_bytes() {
        let tokenizer = tokenizer(&[(b'a', b'b')]);
        let input = b"ab\r\nab\n\n\xff";
        let ids = tokenizer.encode(input, &limits()).unwrap();
        assert_eq!(
            ids,
            [
                BPE_TOKEN_START,
                base_byte_token(b'\r'),
                base_byte_token(b'\n'),
                BPE_TOKEN_START,
                base_byte_token(b'\n'),
                base_byte_token(b'\n'),
                base_byte_token(0xFF),
            ]
        );
        assert_eq!(tokenizer.decode(&ids, &limits()).unwrap(), input);
    }

    #[test]
    fn aliases_collapse_to_shared_ids_and_decode_only_canonical_bytes() {
        let tokenizer = tokenizer(&[]);
        let input = b"<KAREEM></KAREEM><kareem_narration>";
        let ids = tokenizer.encode(input, &limits()).unwrap();
        assert_eq!(ids, [4, 4, 4]);
        let canonical = b"<kareem_narration><kareem_narration><kareem_narration>";
        assert_eq!(tokenizer.decode(&ids, &limits()).unwrap(), canonical);
        let document = EncodedTokensV1::encode(&tokenizer, input, &limits()).unwrap();
        assert_eq!(document.decoded_bytes(), canonical.len() as u64);
        assert_eq!(document.decoded_sha256(), sha256(canonical));
        assert_eq!(document.decode(&tokenizer, &limits()).unwrap(), canonical);
    }

    #[test]
    fn token_ids_above_u16_survive_codec_and_json_as_u32() {
        let tokenizer = cross_u16_tokenizer();
        let input = [254u8, 236u8];
        let document = EncodedTokensV1::encode(&tokenizer, &input, &limits()).unwrap();
        assert_eq!(document.token_ids(), [65_536]);
        let json = document.to_canonical_json(&limits()).unwrap();
        assert!(json.windows(b"65536".len()).any(|bytes| bytes == b"65536"));
        let parsed = EncodedTokensV1::parse_json(&json, &limits()).unwrap();
        assert_eq!(parsed.decode(&tokenizer, &limits()).unwrap(), input);
    }

    #[test]
    fn canonical_json_round_trip_is_stable_bound_and_hash_checked() {
        let tokenizer = ranked_tokenizer().expect("ranked tokenizer");
        let mut merge_payload = Vec::new();
        for ordinal in 0..tokenizer.merge_count() {
            let merge = tokenizer.merge_at(ordinal).expect("merge record");
            merge_payload.extend_from_slice(&merge.a.to_le_bytes());
            merge_payload.extend_from_slice(&merge.b.to_le_bytes());
            merge_payload.extend_from_slice(&merge.merged.to_le_bytes());
        }
        let header = ArtifactHeaderV3::from_merge_payload(279, &merge_payload).unwrap();
        assert_eq!(
            tokenizer_sequence_sha256(&tokenizer).unwrap(),
            header.sequence_sha256()
        );
        let input = b"abc abc\r\n\xff";
        let document = EncodedTokensV1::encode(&tokenizer, input, &limits()).unwrap();
        let json = document.to_canonical_json(&limits()).unwrap();
        assert_eq!(json.last(), Some(&b'\n'));
        assert_eq!(
            EncodedTokensV1::parse_json(&json, &limits())
                .unwrap()
                .to_canonical_json(&limits())
                .unwrap(),
            json
        );
        let parsed = EncodedTokensV1::parse_json(&json, &limits()).unwrap();
        assert_eq!(parsed.decode(&tokenizer, &limits()).unwrap(), input);
        assert_eq!(parsed.token_ids(), document.token_ids());
    }

    #[test]
    fn malformed_unknown_non_u32_and_out_of_range_json_fail_closed() {
        let tokenizer = tokenizer(&[]);
        let valid = EncodedTokensV1::encode(&tokenizer, b"x", &limits())
            .unwrap()
            .to_canonical_json(&limits())
            .unwrap();
        let text = String::from_utf8(valid).unwrap();
        assert_eq!(
            text,
            concat!(
                "{\"schema\":\"ashira_v3_encoded_tokens_v1\",",
                "\"token_id_width\":\"u32\",\"tokenizer\":{\"vocab_size\":276,",
                "\"merge_count\":0,\"sequence_sha256\":",
                "\"E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855\"},",
                "\"decoded_bytes\":1,\"decoded_sha256\":",
                "\"2D711642B726B04401627CA9FBAC32F5C8530FB1903CC4DB02258717921A4881\",",
                "\"token_count\":1,\"token_ids\":[140]}\n"
            )
        );

        assert!(matches!(
            EncodedTokensV1::parse_json(b"{", &limits()),
            Err(CodecError::Json { .. })
        ));
        let unknown = text.replacen("\"token_ids\"", "\"unknown\":0,\"token_ids\"", 1);
        assert!(matches!(
            EncodedTokensV1::parse_json(unknown.as_bytes(), &limits()),
            Err(CodecError::Json { .. })
        ));
        let wrong_schema = text.replacen(ENCODED_TOKENS_SCHEMA, "unknown_schema", 1);
        assert_eq!(
            EncodedTokensV1::parse_json(wrong_schema.as_bytes(), &limits()),
            Err(CodecError::SchemaMismatch)
        );
        let wrong_width = text.replacen(
            "\"token_id_width\":\"u32\"",
            "\"token_id_width\":\"u16\"",
            1,
        );
        assert_eq!(
            EncodedTokensV1::parse_json(wrong_width.as_bytes(), &limits()),
            Err(CodecError::TokenIdWidthMismatch)
        );
        let wrong_count = text.replacen("\"token_count\":1", "\"token_count\":2", 1);
        assert_eq!(
            EncodedTokensV1::parse_json(wrong_count.as_bytes(), &limits()),
            Err(CodecError::TokenCountMismatch)
        );
        let wrong_binding_count = text.replacen("\"merge_count\":0", "\"merge_count\":1", 1);
        assert_eq!(
            EncodedTokensV1::parse_json(wrong_binding_count.as_bytes(), &limits()),
            Err(CodecError::TokenizerBindingMismatch)
        );
        let digest = encode_upper_hex(&sha256(b"x"));
        let lowercase_digest = text.replacen(&digest, &digest.to_ascii_lowercase(), 1);
        assert!(matches!(
            EncodedTokensV1::parse_json(lowercase_digest.as_bytes(), &limits()),
            Err(CodecError::Json { .. })
        ));
        let token_field = format!("\"token_ids\":[{}]", base_byte_token(b'x'));
        let too_wide = text.replacen(&token_field, "\"token_ids\":[4294967296]", 1);
        assert!(matches!(
            EncodedTokensV1::parse_json(too_wide.as_bytes(), &limits()),
            Err(CodecError::Json { .. })
        ));
        let above_max = text.replacen(&token_field, "\"token_ids\":[131072]", 1);
        assert!(matches!(
            EncodedTokensV1::parse_json(above_max.as_bytes(), &limits()),
            Err(CodecError::InvalidTokenId { id: 131_072, .. })
        ));
    }

    #[test]
    fn wrong_tokenizer_digest_and_tampered_decoded_hash_fail_closed() {
        let tokenizer_a = ranked_tokenizer().expect("ranked tokenizer");
        let tokenizer_b = tokenizer(&[(b'x', b'y'), (b'y', b'z'), (b'z', b'x')]);
        let document = EncodedTokensV1::encode(&tokenizer_a, b"abc", &limits()).unwrap();
        assert_eq!(
            document.decode(&tokenizer_b, &limits()),
            Err(CodecError::TokenizerBindingMismatch)
        );

        let json = String::from_utf8(document.to_canonical_json(&limits()).unwrap()).unwrap();
        let digest = encode_upper_hex(&sha256(b"abc"));
        let tampered = json.replacen(&digest, &"0".repeat(64), 1);
        let parsed = EncodedTokensV1::parse_json(tampered.as_bytes(), &limits()).unwrap();
        assert_eq!(
            parsed.decode(&tokenizer_a, &limits()),
            Err(CodecError::DecodedDigestMismatch)
        );
    }

    #[test]
    fn token_and_byte_limits_and_invalid_decoder_ids_fail_closed() {
        let tokenizer = tokenizer(&[]);
        let mut tiny = limits();
        tiny.max_input_bytes = 1;
        assert!(matches!(
            tokenizer.encode(b"ab", &tiny),
            Err(CodecError::ResourceLimitExceeded {
                resource: "input_bytes",
                ..
            })
        ));
        tiny = limits();
        tiny.max_token_count = 1;
        assert!(matches!(
            tokenizer.encode(b"ab", &tiny),
            Err(CodecError::ResourceLimitExceeded {
                resource: "encoded_token_count",
                ..
            })
        ));
        assert!(matches!(
            tokenizer.decode(&[131_072], &limits()),
            Err(CodecError::InvalidTokenId { id: 131_072, .. })
        ));
        assert!(matches!(
            tokenizer.decode(&[276], &limits()),
            Err(CodecError::InvalidTokenId { id: 276, .. })
        ));
        tiny = limits();
        tiny.max_decoded_bytes = 1;
        assert!(matches!(
            tokenizer.decode(&[base_byte_token(b'a'), base_byte_token(b'b')], &tiny),
            Err(CodecError::ResourceLimitExceeded {
                resource: "decoded_bytes",
                ..
            })
        ));

        let document = EncodedTokensV1::encode(&tokenizer, b"x", &limits()).unwrap();
        let json = document.to_canonical_json(&limits()).unwrap();
        tiny = limits();
        tiny.max_encoded_json_bytes = u64::try_from(json.len() - 1).unwrap();
        assert!(matches!(
            document.to_canonical_json(&tiny),
            Err(CodecError::ResourceLimitExceeded {
                resource: "encoded_json_bytes",
                ..
            })
        ));
        assert!(matches!(
            EncodedTokensV1::parse_json(&json, &tiny),
            Err(CodecError::ResourceLimitExceeded {
                resource: "encoded_json_bytes",
                ..
            })
        ));
    }
}
