//! 기존 OS 사용자 데이터의 신뢰 등록으로 고정한 읽기 전용 프로젝트.
//! 프로젝트 경로를 입력받거나 신뢰를 부여하지 않습니다.

use std::path::PathBuf;

use stellar_project::identity::ProjectIdentity;
use stellar_project::trust::TrustStatus;
use stellar_project::trust_store::{os_user_data_base, FileTrustStore};

use crate::core_types::ids::StableId;
use crate::snapshot::{open_snapshot, probe, ProjectSnapshot};

/// 신뢰를 확인할 수 없거나 고정한 프로젝트를 열 수 없음. 경로·내용은 노출하지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustedReadError {
    /// 등록 없음, 모호한 ID, 철회, identity 변경 또는 저장소 읽기 실패.
    NotTrusted,
    /// 승인 프로젝트의 snapshot을 읽을 수 없음.
    Unavailable,
}

/// 신뢰 저장소에서 유일하게 선택한 프로젝트. 매 읽기 전에 신뢰와 identity를 재확인합니다.
#[derive(Debug)]
pub struct TrustedProject {
    identity: ProjectIdentity,
    data_dir: PathBuf,
}

impl TrustedProject {
    /// 기존 앱 데이터 위치에서 ID 하나를 선택합니다. 파일 생성·수정은 없습니다.
    pub fn select(project_id: StableId) -> Result<Self, TrustedReadError> {
        let data_dir = os_user_data_base()
            .filter(|base| base.is_absolute())
            .ok_or(TrustedReadError::NotTrusted)?
            .join(crate::lock::APP_DATA_DIR);
        Self::select_in(project_id, data_dir)
    }

    fn select_in(project_id: StableId, data_dir: PathBuf) -> Result<Self, TrustedReadError> {
        let store = FileTrustStore::open(&data_dir).map_err(|_| TrustedReadError::NotTrusted)?;
        let target = store
            .unique_target(project_id)
            .ok_or(TrustedReadError::NotTrusted)?;
        let identity =
            ProjectIdentity::new(project_id, target).map_err(|_| TrustedReadError::NotTrusted)?;
        if identity.canonical_target() != target || store.status(&identity) != TrustStatus::Trusted
        {
            return Err(TrustedReadError::NotTrusted);
        }
        let selected = Self { identity, data_dir };
        selected.check()?;
        Ok(selected)
    }

    /// 선택한 프로젝트 ID.
    pub fn project_id(&self) -> StableId {
        self.identity.project_id()
    }

    /// 최신 신뢰와 고정 경로의 매니페스트 ID를 확인합니다.
    pub fn check(&self) -> Result<(), TrustedReadError> {
        let store =
            FileTrustStore::open(&self.data_dir).map_err(|_| TrustedReadError::NotTrusted)?;
        let target = self.identity.canonical_target();
        if store.unique_target(self.project_id()) != Some(target)
            || store.status(&self.identity) != TrustStatus::Trusted
            || std::fs::canonicalize(target).ok().as_deref() != Some(target)
        {
            return Err(TrustedReadError::NotTrusted);
        }
        let current = probe(target).map_err(|_| TrustedReadError::Unavailable)?;
        if current.manifest.project.id.parse::<StableId>().ok() != Some(self.project_id()) {
            return Err(TrustedReadError::NotTrusted);
        }
        Ok(())
    }

    /// 한 generation의 snapshot을 엽니다. 열기 전후 확인에 실패하면 내용을 반환하지 않습니다.
    pub fn snapshot(&self) -> Result<ProjectSnapshot, TrustedReadError> {
        self.check()?;
        let snapshot = open_snapshot(self.identity.canonical_target())
            .map_err(|_| TrustedReadError::Unavailable)?;
        if snapshot.project_id() != self.project_id() {
            return Err(TrustedReadError::NotTrusted);
        }
        self.check()?;
        Ok(snapshot)
    }
}
