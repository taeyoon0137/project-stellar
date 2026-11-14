import AppKit

/// 나눠 만들기 단계 하나. `heavy` 단계는 slice의 첫 단계로만 실행합니다.
struct InspectorBuildStep {
    /// 비용 등급. macOS 26에서 popup·segmented·stepper·switch·push 버튼은 처음 크기를 재는 데만 0.3~1.2ms가
    /// 들어(docs/design/ui/inspector.md 8.3절) 다른 단계와 한 slice에 묶으면 2ms를 넘습니다.
    enum Cost {
        case light
        case heavy
    }

    let cost: Cost
    let run: () -> Void

    static func light(_ run: @escaping () -> Void) -> InspectorBuildStep {
        InspectorBuildStep(cost: .light, run: run)
    }

    static func heavy(_ run: @escaping () -> Void) -> InspectorBuildStep {
        InspectorBuildStep(cost: .heavy, run: run)
    }
}

/// 인스펙터 페이지를 메인 스레드에서 짧게 나눠 만드는 작업 대기열(아키텍처 §6.3, 가이드 9장).
///
/// AppKit 컨트롤은 메인 스레드에서만 만들 수 있고, macOS 26에서 popup·segmented 하나를 처음 만들고 재는 데
/// 1ms 안팎이 듭니다. 페이지 하나(컨트롤 30~60개)를 한 번에 만들면 2ms 상한을 넘으므로, 작업을 컨트롤 하나 정도의
/// 단계로 나누고 런 루프 한 번에 `sliceBudget`만큼만 실행한 뒤 입력·그리기에 양보합니다.
///
/// - `.active`: 지금 화면에 필요한 페이지(캐시 미스)와 그 보강. 공통 모드(이벤트 추적·모달 포함)에서 먼저 실행합니다.
/// - `.background`: 예열·제거·폭 변경 뒤 재배치. 기본 모드에서만 실행하므로 드래그·메뉴 추적·창 크기 조절 중에는 멈춥니다.
@MainActor
final class InspectorBuildQueue {
    enum Priority: Int {
        case background = 0
        case active = 1
    }

    private final class Job {
        let key: String
        var priority: Priority
        var steps: [InspectorBuildStep]
        var next = 0
        let afterSlice: (() -> Void)?
        let completion: (() -> Void)?

        init(key: String, priority: Priority, steps: [InspectorBuildStep], afterSlice: (() -> Void)?,
             completion: (() -> Void)?) {
            self.key = key
            self.priority = priority
            self.steps = steps
            self.afterSlice = afterSlice
            self.completion = completion
        }

        var isFinished: Bool { next >= steps.count }
    }

    /// 한 slice에서 이 시간(초)이 지나면 새 단계를 시작하지 않습니다. slice 전체는 대략
    /// `sliceBudget` + 단계 하나 + 배치이고, 이어지는 런 루프 그리기는 그 slice가 붙인 컨트롤만 그립니다.
    /// 런 루프 한 번마다 배치·CA commit 고정 비용(약 0.5~1ms, 창의 뷰 수에 비례)이 들어, 아주 작은 단계만 묶습니다.
    var sliceBudget: TimeInterval = 0.00025
    /// `false`이면 런 루프에 예약하지 않습니다(검사가 `runSlice()`를 직접 부릅니다).
    var schedulesAutomatically = true
    /// 각 slice 끝에 실행 시간에 포함해 부르는 작업(검사: 화면 그리기까지 재기).
    var sliceEpilogue: (() -> Void)?
    /// 최근 slice 실행 시간(ms, 최근 4,096개). 검사·진단용입니다.
    private(set) var sliceDurations: [Double] = []

    private var jobs: [Job] = []
    private var scheduledModes: Set<RunLoop.Mode> = []

    /// 대기 중이거나 진행 중인 작업이 있는지.
    var isEmpty: Bool { jobs.isEmpty }
    /// `.active` 작업이 있는지.
    var hasActiveWork: Bool { jobs.contains { $0.priority == .active } }

    func contains(_ key: String) -> Bool {
        jobs.contains { $0.key == key }
    }

    /// 작업을 넣습니다. 같은 키가 있으면 우선순위만 올립니다. 급한 작업은 맨 앞에 둡니다(가장 최근 요청 먼저).
    func enqueue(
        key: String, priority: Priority, steps: [InspectorBuildStep],
        afterSlice: (() -> Void)? = nil, completion: (() -> Void)? = nil
    ) {
        if jobs.contains(where: { $0.key == key }) {
            if priority == .active { promote(key) }
            return
        }
        let job = Job(key: key, priority: priority, steps: steps, afterSlice: afterSlice, completion: completion)
        if priority == .active { jobs.insert(job, at: 0) } else { jobs.append(job) }
        schedule()
    }

    /// 작업을 급한 작업으로 올리고 맨 앞에 둡니다(예열 중인 페이지가 지금 필요해짐).
    func promote(_ key: String) {
        guard let index = jobs.firstIndex(where: { $0.key == key }) else { return }
        let job = jobs.remove(at: index)
        job.priority = .active
        jobs.insert(job, at: 0)
        schedule()
    }

    /// 키가 `prefix`로 시작하는 급한 작업을 배경 작업으로 낮춥니다(선택이 바뀌어 지금은 필요 없는 페이지).
    /// 만들던 페이지는 버리지 않고 유휴 시간에 마저 만듭니다.
    func demote(keysWithPrefix prefix: String) {
        for job in jobs where job.priority == .active && job.key.hasPrefix(prefix) {
            job.priority = .background
        }
    }

    /// 작업을 뺍니다. 이미 실행한 단계는 되돌리지 않습니다.
    func cancel(_ key: String) {
        jobs.removeAll { $0.key == key }
    }

    /// slice 하나를 실행합니다. 남은 작업이 있으면 true.
    @discardableResult
    func runSlice() -> Bool {
        guard let job = nextJob() else { return false }
        let start = DispatchTime.now().uptimeNanoseconds
        let budget = UInt64(sliceBudget * 1_000_000_000)
        var first = true
        while !job.isFinished {
            let step = job.steps[job.next]
            // 비싼 단계는 slice 첫 단계로만 실행합니다(다른 단계와 묶이면 상한을 넘음).
            if !first && (step.cost == .heavy || DispatchTime.now().uptimeNanoseconds - start >= budget) { break }
            step.run()
            job.next += 1
            first = false
            if step.cost == .heavy { break }
        }
        job.afterSlice?()
        if job.isFinished {
            jobs.removeAll { $0 === job }
            job.completion?()
        }
        sliceEpilogue?()
        record(Double(DispatchTime.now().uptimeNanoseconds - start) / 1_000_000)
        return !jobs.isEmpty
    }

    /// 조건을 만족하거나 대기열이 빌 때까지 slice를 차례로 실행합니다(검사·즉시 완료용).
    func runSlices(while condition: () -> Bool) {
        while condition(), runSlice() {}
    }

    /// `.active` 작업이 없어질 때까지 실행합니다.
    func finishActiveWork() {
        runSlices { hasActiveWork }
    }

    /// 기록한 slice 시간을 비웁니다.
    func resetMeasurements() {
        sliceDurations.removeAll()
    }

    // MARK: - 내부

    private func nextJob() -> Job? {
        jobs.first { $0.priority == .active } ?? jobs.first
    }

    private func record(_ milliseconds: Double) {
        sliceDurations.append(milliseconds)
        if sliceDurations.count > 4096 { sliceDurations.removeFirst(sliceDurations.count - 4096) }
    }

    /// 다음 런 루프에 slice를 예약합니다. 급한 작업은 공통 모드, 예열은 기본 모드에서만 돕니다.
    private func schedule() {
        guard schedulesAutomatically, !jobs.isEmpty else { return }
        let mode: RunLoop.Mode = hasActiveWork ? .common : .default
        guard !scheduledModes.contains(mode) else { return }
        scheduledModes.insert(mode)
        RunLoop.main.perform(inModes: [mode]) { [weak self] in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.scheduledModes.remove(mode)
                // 예열은 기본 모드에서만 실행합니다(공통 모드 예약이 이벤트 추적 중에 돌았다면 급한 작업만).
                if self.hasActiveWork || RunLoop.current.currentMode == .default {
                    self.runSlice()
                }
                self.schedule()
            }
        }
    }
}
