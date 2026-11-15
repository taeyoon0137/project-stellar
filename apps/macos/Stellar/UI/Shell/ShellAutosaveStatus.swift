import AppKit

/// 자동 저장이 멈춘 이유(부제목 앞 경고, UX 명세 3.3 "저장 실패").
enum ShellSaveIssue: Equatable {
    /// 마지막 자동 저장이 실패했습니다. 코어가 다시 시도합니다.
    case failed
    /// 다른 창·프로세스가 먼저 저장해 자동 저장이 멈췄습니다. 이 창의 편집은 메모리에만 있습니다.
    case stale
}

/// 자동 저장 상태 표시(ADR-0072 7절).
///
/// 자동 저장은 요청이 아니라 완료 event가 없으므로 코어 상태 세대를 관찰합니다. 창 info의 `autosave`가 기다리는 상태
/// (`PENDING`·`SAVING`·`FAILED`)인 동안만 event 펌프가 상태 세대를 읽고, 바뀌면 부제목의 저장 경고를 고칩니다. `STALE`은
/// clean이 될 때까지 바뀌지 않으므로 관찰을 멈추고 표시만 남깁니다. 평소 자동 저장은 아무것도 표시하지 않습니다.
extension ShellProjectWindow {
    /// 이 창의 자동 저장 상태 관찰을 시작합니다(창 session을 연 뒤). 이전 관찰은 끝냅니다.
    func observeAutosave() {
        stopObservingAutosave()
        guard let pump = dataSource.session?.pump else { return }
        let token = pump.observeState(isWaiting: { [weak self] in
            guard let state = self?.windowController.windowSession?.info?.autosave else { return false }
            return state == UInt32(STELLAR_AUTOSAVE_PENDING) || state == UInt32(STELLAR_AUTOSAVE_SAVING)
                || state == UInt32(STELLAR_AUTOSAVE_FAILED)
        }, changed: { [weak self] in
            self?.refreshAutosaveStatus()
        })
        autosaveObservation = (pump, token)
        refreshAutosaveStatus()
    }

    /// 관찰을 끝냅니다(창 session을 닫거나 다시 열 때).
    func stopObservingAutosave() {
        guard let (pump, token) = autosaveObservation else { return }
        autosaveObservation = nil
        pump.removeStateObserver(token)
    }

    /// 창 info의 자동 저장 상태를 부제목에 반영합니다.
    func refreshAutosaveStatus() {
        let issue: ShellSaveIssue?
        switch windowController.windowSession?.info?.autosave {
        case UInt32(STELLAR_AUTOSAVE_FAILED): issue = .failed
        case UInt32(STELLAR_AUTOSAVE_STALE): issue = .stale
        default: issue = nil
        }
        dataSource.updateSaveIssue(issue)
    }
}
