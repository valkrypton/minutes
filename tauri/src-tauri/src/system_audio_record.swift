import AVFoundation
import CoreMedia
import Dispatch
import Foundation
import ScreenCaptureKit

@available(macOS 15.0, *)
final class NativeCallRecorder: NSObject, SCRecordingOutputDelegate, SCStreamOutput {
    private var stream: SCStream?
    private var recordingOutput: SCRecordingOutput?
    private let outputURL: URL
    private let sampleQueue = DispatchQueue(label: "minutes.system-audio.samples")
    private let monitorQueue = DispatchQueue(label: "minutes.system-audio.monitor")
    private var monitorTimer: DispatchSourceTimer?
    private var lastSystemAudioSampleAt: Date?
    private var lastMicSampleAt: Date?
    private var lastReportedSystemLive = false
    private var lastReportedMicLive = false

    init(outputURL: URL) {
        self.outputURL = outputURL
    }

    func start() async throws {
        let shareableContent = try await SCShareableContent.excludingDesktopWindows(
            false,
            onScreenWindowsOnly: true
        )
        guard let display = shareableContent.displays.first else {
            throw NSError(
                domain: "MinutesSystemAudioRecord",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "No display available for ScreenCaptureKit capture."]
            )
        }

        let filter = SCContentFilter(
            display: display,
            excludingApplications: [],
            exceptingWindows: []
        )

        let configuration = SCStreamConfiguration()
        configuration.width = 2
        configuration.height = 2
        configuration.minimumFrameInterval = CMTime(value: 1, timescale: 2)
        configuration.queueDepth = 3
        configuration.capturesAudio = true
        configuration.captureMicrophone = true
        configuration.excludesCurrentProcessAudio = true
        configuration.showsCursor = false

        if let microphone = AVCaptureDevice.default(for: .audio) {
            configuration.microphoneCaptureDeviceID = microphone.uniqueID
        }

        let stream = SCStream(filter: filter, configuration: configuration, delegate: nil)
        try stream.addStreamOutput(self, type: .audio, sampleHandlerQueue: sampleQueue)
        if #available(macOS 15.0, *) {
            try stream.addStreamOutput(self, type: .microphone, sampleHandlerQueue: sampleQueue)
        }
        let recordingConfiguration = SCRecordingOutputConfiguration()
        recordingConfiguration.outputURL = outputURL
        recordingConfiguration.outputFileType = .mov
        recordingConfiguration.videoCodecType = .h264

        let recordingOutput = SCRecordingOutput(
            configuration: recordingConfiguration,
            delegate: self
        )

        try stream.addRecordingOutput(recordingOutput)
        try await stream.startCapture()

        startMonitoring()

        self.stream = stream
        self.recordingOutput = recordingOutput
    }

    func stop() async {
        guard let stream else {
            exit(0)
        }

        do {
            try await stream.stopCapture()
        } catch {
            fputs("stopCapture failed: \(error)\n", stderr)
            exit(1)
        }
    }

    private func startMonitoring() {
        let timer = DispatchSource.makeTimerSource(queue: monitorQueue)
        timer.schedule(deadline: .now(), repeating: .milliseconds(500))
        timer.setEventHandler { [weak self] in
            guard let self else { return }
            let now = Date()
            let systemLive = self.lastSystemAudioSampleAt.map { now.timeIntervalSince($0) < 1.5 } ?? false
            let micLive = self.lastMicSampleAt.map { now.timeIntervalSince($0) < 1.5 } ?? false
            if systemLive != self.lastReportedSystemLive || micLive != self.lastReportedMicLive {
                self.lastReportedSystemLive = systemLive
                self.lastReportedMicLive = micLive
                let payload: [String: Any] = [
                    "event": "health",
                    "call_audio_live": systemLive,
                    "mic_live": micLive
                ]
                if let data = try? JSONSerialization.data(withJSONObject: payload),
                   let json = String(data: data, encoding: .utf8) {
                    print(json)
                    fflush(stdout)
                }
            }
        }
        timer.resume()
        monitorTimer = timer
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of outputType: SCStreamOutputType) {
        guard CMSampleBufferIsValid(sampleBuffer), CMSampleBufferDataIsReady(sampleBuffer) else {
            return
        }
        let now = Date()
        switch outputType {
        case .audio:
            lastSystemAudioSampleAt = now
        case .microphone:
            lastMicSampleAt = now
        default:
            break
        }
    }

    func recordingOutputDidStartRecording(_ recordingOutput: SCRecordingOutput) {
        print("ready")
        fflush(stdout)
    }

    func recordingOutputDidFinishRecording(_ recordingOutput: SCRecordingOutput) {
        exit(0)
    }

    func recordingOutput(
        _ recordingOutput: SCRecordingOutput,
        didFailWithError error: Error
    ) {
        fputs("recordingOutput failed: \(error)\n", stderr)
        exit(1)
    }
}

@main
struct NativeCallRecordMain {
    static func main() {
        Task {
            await run()
        }
        dispatchMain()
    }

    static func run() async {
        guard #available(macOS 15.0, *) else {
            fputs("ScreenCaptureKit recording output requires macOS 15.0 or newer.\n", stderr)
            exit(1)
        }

        guard CommandLine.arguments.count >= 2 else {
            fputs("usage: system_audio_record <output.mov>\n", stderr)
            exit(1)
        }

        let outputURL = URL(fileURLWithPath: CommandLine.arguments[1])
        let recorder = NativeCallRecorder(outputURL: outputURL)

        signal(SIGTERM, SIG_IGN)
        let stopSource = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
        stopSource.setEventHandler {
            Task {
                await recorder.stop()
            }
        }
        stopSource.resume()

        do {
            try await recorder.start()
        } catch {
            fputs("start failed: \(error)\n", stderr)
            exit(1)
        }
    }
}
