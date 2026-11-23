//! canonical recovery payload (V.9.2).
//!
//! 복구 스냅샷 파일(V.9.1)의 내용 형식입니다. 포함하는 것:
//!
//! - 문서 미저장 snapshot(base 세대 + 미저장 logical chunk payload들, revision)
//! - 열린 탭 목록(view id + viewport)과 활성 탭
//! - 선택된 node 목록
//! - target profile
//!
//! **포함하지 않는 것**: 터미널 상태, Git 실행 상태 — 복구 대상이 아닙니다.
//!
//! 인코딩은 canonical 규칙(V.1)을 따릅니다: 고정 필드 순서, LE 고정 폭,
//! 길이 프리픽스, 유한 float(비유한 viewport 거부, `-0.0` 정규화).

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_storage::canonical::decode::{CanonicalDecoder, DecodeError};
use stellar_storage::canonical::encode::{CanonicalEncoder, EncodeError};
use stellar_storage::chunk::envelope::ChunkKindCode;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::hash::ContentHash;

/// recovery payload 스키마 버전(파일 첫 필드).
pub const RECOVERY_PAYLOAD_VERSION: u32 = 1;

/// 미저장 logical chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsavedChunk {
    /// 논리 chunk id.
    pub chunk_id: ChunkId,
    /// chunk kind.
    pub kind: ChunkKindCode,
    /// canonical payload bytes.
    pub payload: Vec<u8>,
}

/// 한 탭의 viewport(유한 값만 허용).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// 화면 중심 x(문서 좌표).
    pub center_x: f64,
    /// 화면 중심 y(문서 좌표).
    pub center_y: f64,
    /// 배율.
    pub zoom: f64,
}

/// 열린 탭.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenTab {
    /// 탭이 보여주는 view의 StableId.
    pub view: StableId,
    /// 탭의 viewport.
    pub viewport: Viewport,
}

/// canonical recovery payload.
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryPayload {
    /// snapshot이 기반한 마지막 정상 저장 세대의 manifest hash(무저장이면 None).
    pub base_manifest_hash: Option<ContentHash>,
    /// 문서 revision(단조 증가). V.9.3 최신성 비교에 사용합니다.
    pub revision: u64,
    /// 미저장 logical chunk들.
    pub unsaved_chunks: Vec<UnsavedChunk>,
    /// 열린 탭 목록(표시 순).
    pub open_tabs: Vec<OpenTab>,
    /// 활성 탭 index(`open_tabs`가 비어 있으면 0).
    pub active_tab: u32,
    /// 선택된 node StableId들.
    pub selection: Vec<StableId>,
    /// target profile id.
    pub target_profile: StableId,
}

/// payload 인코딩·디코딩 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryPayloadError {
    /// 지원하지 않는 payload 버전.
    UnsupportedVersion {
        /// 파일이 선언한 버전.
        found: u32,
    },
    /// 활성 탭 index가 탭 수를 벗어납니다.
    ActiveTabOutOfRange {
        /// 활성 index.
        active: u32,
        /// 탭 수.
        tabs: usize,
    },
    /// canonical 인코딩 실패(비유한 viewport 등).
    Encode(EncodeError),
    /// canonical 디코딩 실패.
    Decode(DecodeError),
}

impl fmt::Display for RecoveryPayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecoveryPayloadError::UnsupportedVersion { found } => {
                write!(f, "지원하지 않는 recovery payload 버전: {found}")
            }
            RecoveryPayloadError::ActiveTabOutOfRange { active, tabs } => {
                write!(f, "활성 탭 index {active}가 탭 수 {tabs}를 벗어납니다")
            }
            RecoveryPayloadError::Encode(e) => write!(f, "payload 인코딩 실패: {e}"),
            RecoveryPayloadError::Decode(e) => write!(f, "payload 디코딩 실패: {e}"),
        }
    }
}

impl std::error::Error for RecoveryPayloadError {}

impl From<EncodeError> for RecoveryPayloadError {
    fn from(e: EncodeError) -> RecoveryPayloadError {
        RecoveryPayloadError::Encode(e)
    }
}

impl From<DecodeError> for RecoveryPayloadError {
    fn from(e: DecodeError) -> RecoveryPayloadError {
        RecoveryPayloadError::Decode(e)
    }
}

impl RecoveryPayload {
    /// 구조 불변식을 검증합니다.
    pub fn validate(&self) -> Result<(), RecoveryPayloadError> {
        if self.open_tabs.is_empty() {
            if self.active_tab != 0 {
                return Err(RecoveryPayloadError::ActiveTabOutOfRange {
                    active: self.active_tab,
                    tabs: 0,
                });
            }
        } else if self.active_tab as usize >= self.open_tabs.len() {
            return Err(RecoveryPayloadError::ActiveTabOutOfRange {
                active: self.active_tab,
                tabs: self.open_tabs.len(),
            });
        }
        Ok(())
    }

    /// canonical bytes로 인코딩합니다.
    pub fn encode(&self) -> Result<Vec<u8>, RecoveryPayloadError> {
        self.validate()?;
        let mut enc = CanonicalEncoder::new();
        enc.put_u32(RECOVERY_PAYLOAD_VERSION);

        match &self.base_manifest_hash {
            Some(hash) => {
                enc.put_bool(true);
                enc.put_string(&hash.to_string())?;
            }
            None => enc.put_bool(false),
        }
        enc.put_u64(self.revision);

        enc.put_u32(u32::try_from(self.unsaved_chunks.len()).map_err(|_| {
            RecoveryPayloadError::Encode(EncodeError::LengthOverflow {
                len: self.unsaved_chunks.len(),
            })
        })?);
        for chunk in &self.unsaved_chunks {
            enc.put_fixed_raw(&chunk.chunk_id.to_bytes());
            enc.put_u32(chunk.kind.0);
            enc.put_bytes(&chunk.payload)?;
        }

        enc.put_u32(u32::try_from(self.open_tabs.len()).map_err(|_| {
            RecoveryPayloadError::Encode(EncodeError::LengthOverflow {
                len: self.open_tabs.len(),
            })
        })?);
        for tab in &self.open_tabs {
            enc.put_fixed_raw(&tab.view.to_bytes());
            enc.put_f64(tab.viewport.center_x)?;
            enc.put_f64(tab.viewport.center_y)?;
            enc.put_f64(tab.viewport.zoom)?;
        }
        enc.put_u32(self.active_tab);

        enc.put_u32(u32::try_from(self.selection.len()).map_err(|_| {
            RecoveryPayloadError::Encode(EncodeError::LengthOverflow {
                len: self.selection.len(),
            })
        })?);
        for id in &self.selection {
            enc.put_fixed_raw(&id.to_bytes());
        }

        enc.put_fixed_raw(&self.target_profile.to_bytes());
        Ok(enc.into_bytes())
    }

    /// canonical bytes에서 디코딩하고 검증합니다.
    pub fn decode(bytes: &[u8]) -> Result<RecoveryPayload, RecoveryPayloadError> {
        let mut dec = CanonicalDecoder::new(bytes);
        let version = dec.take_u32()?;
        if version != RECOVERY_PAYLOAD_VERSION {
            return Err(RecoveryPayloadError::UnsupportedVersion { found: version });
        }

        let base_manifest_hash = if dec.take_bool()? {
            let text = dec.take_string()?;
            Some(
                text.parse::<ContentHash>()
                    .map_err(|_| RecoveryPayloadError::Decode(DecodeError::InvalidUtf8))?,
            )
        } else {
            None
        };
        let revision = dec.take_u64()?;

        let chunk_count = dec.take_u32()? as usize;
        let mut unsaved_chunks = Vec::with_capacity(chunk_count.min(1024));
        for _ in 0..chunk_count {
            let mut id = [0u8; 16];
            id.copy_from_slice(dec.take_raw(16)?);
            let kind = ChunkKindCode(dec.take_u32()?);
            let payload = dec.take_bytes()?.to_vec();
            unsaved_chunks.push(UnsavedChunk {
                chunk_id: ChunkId::from_bytes(id),
                kind,
                payload,
            });
        }

        let tab_count = dec.take_u32()? as usize;
        let mut open_tabs = Vec::with_capacity(tab_count.min(1024));
        for _ in 0..tab_count {
            let mut id = [0u8; 16];
            id.copy_from_slice(dec.take_raw(16)?);
            let center_x = dec.take_f64()?;
            let center_y = dec.take_f64()?;
            let zoom = dec.take_f64()?;
            open_tabs.push(OpenTab {
                view: StableId::from_bytes(id),
                viewport: Viewport {
                    center_x,
                    center_y,
                    zoom,
                },
            });
        }
        let active_tab = dec.take_u32()?;

        let selection_count = dec.take_u32()? as usize;
        let mut selection = Vec::with_capacity(selection_count.min(4096));
        for _ in 0..selection_count {
            let mut id = [0u8; 16];
            id.copy_from_slice(dec.take_raw(16)?);
            selection.push(StableId::from_bytes(id));
        }

        let mut profile = [0u8; 16];
        profile.copy_from_slice(dec.take_raw(16)?);
        dec.expect_end()?;

        let payload = RecoveryPayload {
            base_manifest_hash,
            revision,
            unsaved_chunks,
            open_tabs,
            active_tab,
            selection,
            target_profile: StableId::from_bytes(profile),
        };
        payload.validate()?;
        Ok(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn sample() -> RecoveryPayload {
        RecoveryPayload {
            base_manifest_hash: Some(ContentHash::compute(b"base-manifest")),
            revision: 42,
            unsaved_chunks: vec![UnsavedChunk {
                chunk_id: ChunkId::from_stable_id(stable(1_000)),
                kind: ChunkKindCode(0x8000_0003),
                payload: b"dirty-view-tree".to_vec(),
            }],
            open_tabs: vec![
                OpenTab {
                    view: stable(2_000),
                    viewport: Viewport {
                        center_x: 120.5,
                        center_y: -44.25,
                        zoom: 1.5,
                    },
                },
                OpenTab {
                    view: stable(2_001),
                    viewport: Viewport {
                        center_x: 0.0,
                        center_y: 0.0,
                        zoom: 0.5,
                    },
                },
            ],
            active_tab: 1,
            selection: vec![stable(3_000), stable(3_001)],
            target_profile: stable(4_000),
        }
    }

    #[test]
    fn payload_round_trips() {
        // 검증 기준(V.9.2): payload round-trip.
        let payload = sample();
        let bytes = payload.encode().unwrap();
        let decoded = RecoveryPayload::decode(&bytes).unwrap();
        assert_eq!(decoded, payload);
        // 재인코딩 byte-identical(canonical).
        assert_eq!(decoded.encode().unwrap(), bytes);
    }

    #[test]
    fn minimal_payload_round_trips() {
        let payload = RecoveryPayload {
            base_manifest_hash: None,
            revision: 0,
            unsaved_chunks: vec![],
            open_tabs: vec![],
            active_tab: 0,
            selection: vec![],
            target_profile: stable(4_000),
        };
        let bytes = payload.encode().unwrap();
        assert_eq!(RecoveryPayload::decode(&bytes).unwrap(), payload);
    }

    #[test]
    fn non_finite_viewport_is_rejected() {
        let mut payload = sample();
        payload.open_tabs[0].viewport.zoom = f64::NAN;
        assert!(matches!(
            payload.encode().unwrap_err(),
            RecoveryPayloadError::Encode(_)
        ));
    }

    #[test]
    fn active_tab_out_of_range_is_rejected() {
        let mut payload = sample();
        payload.active_tab = 2; // 탭 2개, index 2는 범위 밖.
        assert!(matches!(
            payload.encode().unwrap_err(),
            RecoveryPayloadError::ActiveTabOutOfRange { active: 2, tabs: 2 }
        ));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let mut bytes = sample().encode().unwrap();
        bytes[0] = 99; // version LE 첫 바이트.
        assert!(matches!(
            RecoveryPayload::decode(&bytes).unwrap_err(),
            RecoveryPayloadError::UnsupportedVersion { found: 99 }
        ));
    }

    #[test]
    fn truncated_payload_is_rejected() {
        let bytes = sample().encode().unwrap();
        for cut in [0, 4, 5, 20, bytes.len() - 1] {
            assert!(
                RecoveryPayload::decode(&bytes[..cut]).is_err(),
                "절단 {cut}"
            );
        }
        // trailing bytes 거부.
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(RecoveryPayload::decode(&extended).is_err());
    }

    #[test]
    fn payload_has_no_terminal_output_or_process_fields() {
        // XVIII.4.6, ADR-0023 결정 10: recovery snapshot에 터미널 출력·프로세스를 넣지 않습니다. 필드가 늘면 이
        // 구조 분해가 컴파일되지 않아 이 검사를 다시 보게 됩니다.
        let RecoveryPayload {
            base_manifest_hash: _,
            revision: _,
            unsaved_chunks: _,
            open_tabs: _,
            active_tab: _,
            selection: _,
            target_profile: _,
        } = sample();
        let OpenTab {
            view: _,
            viewport: _,
        } = sample().open_tabs[0].clone();
        // payload를 만들고 읽는 crate(project·runtime)는 터미널 crate에 의존하지 않습니다. 복구 문서 chunk는
        // runtime이 문서 직렬화 결과와 같은지 다시 확인하므로 다른 내용이 끼어들 수 없습니다.
        for (name, manifest) in [
            ("stellar-project", include_str!("../Cargo.toml")),
            ("stellar-runtime", include_str!("../../runtime/Cargo.toml")),
        ] {
            assert!(!manifest.contains("stellar-terminal"), "{name}");
        }
    }
}
