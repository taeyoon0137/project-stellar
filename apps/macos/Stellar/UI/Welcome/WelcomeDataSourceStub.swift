import Foundation

/// 코어 연결 전 시작 창용 표본 데이터 공급자(의도적 placeholder, 계획 1.2 규칙 6).
///
/// 실제 프로젝트를 만들거나 열지 않습니다. 받은 명령을 `sentCommands`에 기록하고, 화면 상태 확인에
/// 필요한 최소 변화(최근 목록 제거, 안내 확인, 작업 취소)만 흉내 냅니다. 코어 어댑터가 생기면 교체합니다.
final class WelcomeDataSourceStub: WelcomeDataSource {
    /// 미리 준비한 화면 상태.
    enum Scenario {
        /// 최근 프로젝트 몇 개(없어진 폴더 하나 포함), Git 사용 가능.
        case sample
        /// 최근 프로젝트 없음, Git 사용 가능.
        case empty
        /// 최근 목록을 읽는 중, Git 탐색 중.
        case loading
        /// 최근 목록 읽기 실패.
        case failed
        /// 최근 프로젝트 있음, Git 없음.
        case gitMissing
        /// Clone 진행 중.
        case cloning
    }

    var snapshotDidChange: (() -> Void)?

    /// 받은 명령 기록(검사용).
    private(set) var sentCommands: [WelcomeCommand] = []

    private var snapshot: WelcomeSnapshot
    private var nextNoticeID: UInt64 = 1

    init(scenario: Scenario = .sample, homeDirectory: String = NSHomeDirectory()) {
        let projects = WelcomeDataSourceStub.sampleProjects(homeDirectory: homeDirectory)
        var snapshot = WelcomeSnapshot(
            generation: 1,
            recentProjects: .loaded(projects),
            git: .available(version: "2.50.1"),
            activity: nil,
            notice: nil,
            defaultParentDirectory: nil
        )
        switch scenario {
        case .sample:
            break
        case .empty:
            snapshot.recentProjects = .loaded([])
        case .loading:
            snapshot.recentProjects = .loading
            snapshot.git = .checking
        case .failed:
            snapshot.recentProjects = .failed(.permissionDenied(path: "~/Library/Application Support/Stellar"))
        case .gitMissing:
            snapshot.git = .notFound
        case .cloning:
            snapshot.activity = WelcomeActivity(
                id: 7,
                kind: .cloning,
                projectName: "design-system",
                fractionCompleted: 0.4,
                isCancellable: true
            )
        }
        self.snapshot = snapshot
    }

    func currentSnapshot() -> WelcomeSnapshot {
        snapshot
    }

    /// 검사용: snapshot을 직접 바꾸고 변경을 알립니다.
    func replaceSnapshot(_ update: (inout WelcomeSnapshot) -> Void) {
        update(&snapshot)
        snapshot.generation += 1
        snapshotDidChange?()
    }

    func send(_ command: WelcomeCommand) {
        sentCommands.append(command)
        switch command {
        case .removeRecent(let id):
            guard case .loaded(var projects) = snapshot.recentProjects else { return }
            projects.removeAll { $0.id == id }
            replaceSnapshot { $0.recentProjects = .loaded(projects) }
        case .openRecent(let id):
            guard case .loaded(let projects) = snapshot.recentProjects,
                let project = projects.first(where: { $0.id == id }),
                project.isMissing
            else { return }
            postNotice(.projectMissing(path: project.path))
        case .reloadRecentProjects:
            let projects = WelcomeDataSourceStub.sampleProjects(homeDirectory: NSHomeDirectory())
            replaceSnapshot { $0.recentProjects = .loaded(projects) }
        case .cancelActivity(let id):
            // 끝난 작업의 늦은 취소는 무시합니다(식별자 비교).
            guard snapshot.activity?.id == id else { return }
            replaceSnapshot { $0.activity = nil }
        case .dismissNotice(let id):
            guard snapshot.notice?.id == id else { return }
            replaceSnapshot { $0.notice = nil }
        case .createProject, .openProject, .cloneProject, .recheckGit, .useGitExecutable:
            // 코어 연결 전에는 기록만 합니다.
            break
        }
    }

    private func postNotice(_ kind: WelcomeNotice.Kind) {
        let notice = WelcomeNotice(id: nextNoticeID, kind: kind)
        nextNoticeID += 1
        replaceSnapshot { $0.notice = notice }
    }

    /// 표본 최근 프로젝트. 경로는 사용자 홈 아래 가상 위치입니다.
    static func sampleProjects(homeDirectory: String) -> [WelcomeRecentProject] {
        let base = (homeDirectory as NSString).appendingPathComponent("Projects")
        func project(_ id: String, _ name: String, _ folder: String, missing: Bool = false)
            -> WelcomeRecentProject
        {
            WelcomeRecentProject(
                id: id,
                displayName: name,
                path: (base as NSString).appendingPathComponent(folder),
                isMissing: missing
            )
        }
        return [
            project("stub-1", "Design System", "design-system"),
            project("stub-2", "Marketing Site", "marketing-site"),
            project("stub-3", "Mobile App", "mobile-app"),
            project("stub-4", "Archived Prototype", "archived-prototype", missing: true),
        ]
    }
}
